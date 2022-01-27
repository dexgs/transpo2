use crate::concurrency::Accessors;
use crate::multipart_form::{self, *};
use trillium::Conn;
use smol::prelude::*;

const BOUNDARY_MAX_LEN: usize = 70;
const EXPECTED_BOUNDARY_START: &'static str = "\r\n-----------------------";


// Content-Disposition for valid form fields
const SERVER_SIDE_PROCESSING_CD: &'static str = "form-data; name=\"server-side-processing\"";
const FILES_CD_PREFIX: &'static str = "form-data; name=\"files\"; filename=";
const DAYS_CD: &'static str = "form-data; name=\"days\"";
const HOURS_CD: &'static str = "form-data; name=\"hours\"";
const MINUTES_CD: &'static str = "form-data; name=\"minutes\"";
const ENABLE_MAX_DOWNLOADS_CD: &'static str = "form-data; name=\"enable-max-downloads\"";
const MAX_DOWNLOADS_CD: &'static str = "form-data; name=\"max-downloads\"";
const ENABLE_PASSWORD_CD: &'static str = "form-data; name=\"enable-password\"";
const PASSWORD_CD: &'static str = "form-data; name=\"password\"";

const VALUE_ON: &'static str = "on";

#[derive(PartialEq, Debug)]
enum FormField {
    ServerSideProcessing,
    Files,
    Days,
    Hours,
    Minutes,
    EnableMaxDownloads,
    MaxDownloads,
    EnablePassword,
    Password,
    Invalid
}

fn match_content_disposition(cd: &str) -> FormField {
    if cd.starts_with(FILES_CD_PREFIX) {
        FormField::Files
    } else {
        match cd {
            SERVER_SIDE_PROCESSING_CD => FormField::ServerSideProcessing,
            DAYS_CD => FormField::Days,
            HOURS_CD => FormField::Hours,
            MINUTES_CD => FormField::Minutes,
            ENABLE_MAX_DOWNLOADS_CD => FormField::EnableMaxDownloads,
            MAX_DOWNLOADS_CD => FormField::MaxDownloads,
            ENABLE_PASSWORD_CD => FormField::EnablePassword,
            PASSWORD_CD => FormField::Password,
            _ => FormField::Invalid
        }
    }
}

#[derive(Debug)]
struct UploadForm {
    server_side_processing: Option<bool>,
    days: Option<usize>,
    hours: Option<usize>,
    minutes: Option<usize>,
    enable_max_downloads: Option<bool>,
    max_downloads: Option<usize>,
    enable_password: Option<bool>,
    password: Option<String>
}

impl UploadForm {
    fn new() -> Self {
        Self {
            server_side_processing: None,
            days: None,
            hours: None,
            minutes: None,
            enable_max_downloads: None,
            max_downloads: None,
            enable_password: None,
            password: None
        }
    }

    fn is_valid_field(&self, field: &FormField) -> bool {
        match field {
            FormField::ServerSideProcessing => self.server_side_processing.is_none(),
            FormField::Days => self.days.is_none(),
            FormField::Hours => self.hours.is_none(),
            FormField::Minutes => self.minutes.is_none(),
            FormField::EnableMaxDownloads => self.enable_max_downloads.is_none(),
            FormField::MaxDownloads => self.max_downloads.is_none(),
            FormField::EnablePassword => self.enable_password.is_none(),
            FormField::Password => self.password.is_none(),
            _ => false
        }
    }

    // Parses the given form field and returns whether or not the input is valid
    fn parse_field(&mut self, field: &FormField, value: &[u8]) -> bool {
        match std::str::from_utf8(value) {
            Ok(value) => {
                match field {
                    FormField::ServerSideProcessing => Self::parse_bool_value(value, &mut self.server_side_processing),
                    FormField::Days => Self::parse_usize_value(value, &mut self.days),
                    FormField::Hours => Self::parse_usize_value(value, &mut self.hours),
                    FormField::Minutes => Self::parse_usize_value(value, &mut self.minutes),
                    FormField::EnableMaxDownloads => Self::parse_bool_value(value, &mut self.enable_max_downloads),
                    FormField::MaxDownloads => Self::parse_usize_value(value, &mut self.max_downloads),
                    FormField::EnablePassword => Self::parse_bool_value(value, &mut self.enable_password),
                    FormField::Password => Self::parse_string_value(value, &mut self.password),
                    _ => false
                }
            },
            Err(_) => false
        }
    }

    fn parse_bool_value(value: &str, field: &mut Option<bool>) -> bool {
        match *field {
            Some(_) => false,
            None => {
                *field = Some(value == VALUE_ON);
                true
            }
        }
    }

    fn parse_usize_value(value: &str, field: &mut Option<usize>) -> bool {
        match *field {
            Some(_) => false,
            None => match value.parse::<usize>() {
                Ok(value) => {
                    *field = Some(value);
                    true
                },
                Err(_) => false
            }
        }
    }
    
    fn parse_string_value(value: &str, field: &mut Option<String>) -> bool {
        match *field {
            Some(_) => false,
            None => {
                *field = Some(String::from(value));
                true
            }
        }
    }
}


pub async fn handle(mut conn: Conn, max_upload_size: usize, accessors: Accessors) -> Conn {
    // Get the boundary of the multi-part form
    let boundary = match get_boundary(&conn) {
        Some(boundary) => boundary,
        None => return error_400(conn)
    };
    let boundary = format!("\r\n--{}", boundary);
    if boundary.len() > BOUNDARY_MAX_LEN
    || !boundary.starts_with(EXPECTED_BOUNDARY_START)
    {
        // This is unlikely to happen unless someone is trying to abuse the
        // slowest path in the parser, a long boundary that contains ever
        // possible byte value.
        return error_400(conn);
    }
    let boundary_byte_map = byte_map(boundary.as_bytes());

    let mut req_body = conn.request_body().await;

    // Form fields
    let mut form = UploadForm::new();

    let mut upload_success = false;

    let mut buf = [0; 5120];
    // Make the first boundary start with a newline to simplify parsing
    (&mut buf[..2]).copy_from_slice(b"\r\n");
    let mut total_bytes = 0;
    let mut read_start = 2;

    let mut field_type = FormField::Invalid;
    // Form fields other than files are expected to fit in this buffer. If they
    // do not, error 400 will be returned.
    let mut field_buf = [0; 512];
    let mut field_write_start = 0;

    'outer: while let Ok(bytes_read) = req_body.read(&mut buf[read_start..]).await {
        if bytes_read == 0 {
            break;
        } else {
            total_bytes += bytes_read;
        }

        // Respect the maximum upload size
        if total_bytes > max_upload_size {
            break;
        }

        // Parse over the buffer until either parsing ends, or we run out of data
        // i.e. we hit either the end of the buffer or a string of bytes that may
        // or may not be a boundary and we can't be sure until we read more data
        let mut parse_start = 0;
        while buf.len() - parse_start > boundary.len() {
            let parse_result = multipart_form::parse(
                &buf[parse_start..], &boundary, &boundary_byte_map);
            match parse_result {
                // The start of a new field in the form
                ParseResult::NewValue(b, cd, _ct, val) => {
                    parse_start += b;
                    //println!("\nContent-Disposition: `{}`\nContent-Type: `{}`", cd, ct);

                    // parse the value of the previous field
                    if field_type != FormField::Files && field_type != FormField::Invalid {
                        let parse_field_success =
                            form.parse_field(&field_type, &field_buf[..field_write_start]);
                        if !parse_field_success { break 'outer; }
                    }

                    // handle the new field
                    let new_field_type = match_content_disposition(cd);
                    match new_field_type {
                        FormField::Invalid => break 'outer,
                        FormField::Files => {
                            // handle files
                        },
                        _ => {
                            if form.is_valid_field(&new_field_type)
                            && val.len() <= field_buf.len()
                            {
                                // copy new data into the field buffer
                                (&mut field_buf[..val.len()]).copy_from_slice(val);
                                field_write_start = val.len();
                            } else {
                                break 'outer;
                            }
                        }
                    }

                    field_type = new_field_type;
                },
                // The continuation of the value of the previous field
                ParseResult::Continue(b, val) => {
                    parse_start += b;

                    match field_type {
                        FormField::Files => {
                            // handle files
                        },
                        _ => {
                            if field_write_start + val.len() <= field_buf.len() {
                                // copy new data into the field buffer
                                (&mut field_buf[field_write_start..][..val.len()])
                                    .copy_from_slice(val);
                                field_write_start += val.len();
                            } else {
                                break 'outer;
                            }
                        }
                    }
                },
                // The end of the form
                ParseResult::Finished => {
                    // parse the value of the previous field
                    if field_type != FormField::Files && field_type != FormField::Invalid {
                        let parse_field_success = form.parse_field(&field_type, &field_buf[..field_write_start]);
                        if !parse_field_success {
                            break 'outer;
                        }
                    }
                    upload_success = true;
                    break 'outer;
                },
                // An error
                ParseResult::Error => break 'outer,

            }
        }

        // The buffer may contain incomplete data at the end, so we copy it to
        // the front of the buffer and make sure it doesn't get read over
        buf.copy_within(parse_start.., 0);
        read_start = buf.len() - parse_start;
    }

    //println!("\nUPLOAD SIZE: {}\n", total_bytes);
    //println!("{:?}", form);

    if upload_success {
    } else {
        return error_400(conn);
    }

    conn.ok("blah!")
}


// Read the multipart form boundary out of the headers
fn get_boundary<'a>(conn: &'a Conn) -> Option<&'a str> {
    conn.headers()
        .get_str("Content-Type")
        .and_then(|ct| ct.split_once("boundary"))
        .and_then(|(_, boundary)| boundary.split_once('='))
        .and_then(|(_, boundary)| {
            let boundary = boundary.trim();
            if boundary.starts_with('"') {
                let len = boundary.len();
                if len > 1 {
                    Some(&boundary[1..(len - 1)])
                } else {
                    None
                }
            } else {
                Some(boundary)
            }
        })
}


// Set `conn` to contain a 400 error
fn error_400(conn: Conn) -> Conn {
    conn.with_body("Error 400").with_status(400).halt()
}
