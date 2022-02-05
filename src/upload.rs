use crate::concurrency::Accessors;
use crate::multipart_form::{self, *};
use crate::random_bytes::*;
use crate::b64;
use crate::files::*;
use crate::constants::*;
use std::fs;
use std::str;
use std::io::{Result, Error, ErrorKind};
use std::sync::Arc;
use std::path::PathBuf;
use trillium::Conn;
use smol::prelude::*;
use blocking::unblock;
use diesel::prelude::*;


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


pub async fn handle<C>(
    mut conn: Conn, max_upload_size: usize, accessors: Accessors,
    connection: C, storage_path: Arc<PathBuf>) -> Conn
where C: Connection
{
    // Get the boundary of the multi-part form
    let boundary = match get_boundary(&conn) {
        Some(boundary) => boundary,
        None => return error_400(conn)
    };
    let boundary = format!("\r\n--{}", boundary);
    if boundary.len() > MAX_FORM_BOUNDARY_LENGTH
    || !boundary.starts_with(EXPECTED_BOUNDARY_START)
    {
        // This is unlikely to happen unless someone is trying to abuse the
        // slowest path in the parser: a long boundary that contains every
        // possible byte value.
        return error_400(conn);
    }
    let boundary_byte_map = byte_map(boundary.as_bytes());

    let (upload_id, upload_dir) = {
        let accessors = accessors.clone();
        unblock(move || loop {
            if let Ok(id) = str::from_utf8(&b64::base64_encode(&random_bytes(6))) {
                let id = id.to_owned();
                if accessors.increment(id.clone(), true) {
                    let dir = storage_path.join(&id);
                    if fs::create_dir_all(&dir).is_ok() {
                        accessors.decrement(&id);
                        break (id, dir);
                    }
                }
            }
        })
    }.await;
    let upload_path = upload_dir.join("upload");

    let mut file_writer = Writer::None;
    let mut key: Option<Vec<u8>> = None;
    let mut file_name: Option<Vec<u8>> = None;
    let mut mime_type: Option<Vec<u8>> = None;

    // Form fields
    let mut form = UploadForm::new();

    let mut upload_success = false;
    let mut buf = [0; FORM_READ_BUFFER_SIZE];
    let mut req_body = conn.request_body().await;
    // Make the first boundary start with a newline to simplify parsing
    (&mut buf[..2]).copy_from_slice(b"\r\n");
    let mut total_bytes = 0;
    let mut read_start = 2;

    let mut field_type = FormField::Invalid;
    // Form fields other than files are expected to fit in this buffer. If they
    // do not, error 400 will be returned.
    let mut field_buf = [0; FORM_FIELD_BUFFER_SIZE];
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

        // Make sure buf does not contain data from the previous read
        let buf = &mut buf[..(bytes_read + read_start)];

        // Parse over the buffer until either parsing ends, or we run out of data
        // i.e. we hit either the end of the buffer or a string of bytes that may
        // or may not be a boundary and we can't be sure until we read more data
        let mut parse_start = 0;
        while buf.len() - parse_start > boundary.len() {
            let parse_result = multipart_form::parse(
                &buf[parse_start..], &boundary, &boundary_byte_map);
            match parse_result {
                // The start of a new field in the form
                ParseResult::NewValue(b, cd, ct, val) => {
                    parse_start += b;

                    // parse the value of the previous field
                    if field_type != FormField::Files && field_type != FormField::Invalid {
                        if !form.parse_field(&field_type, &field_buf[..field_write_start]) {
                            break 'outer;
                        }
                    }

                    // handle the new field
                    let new_field_type = match_content_disposition(cd);
                    match new_field_type {
                        FormField::Invalid => break 'outer,
                        FormField::Files => {
                            if file_writer.is_none() {
                                let server_side_processing = match form.server_side_processing {
                                    None | Some(false) => false,
                                    Some(true) => true
                                };

                                match handle_file_start(cd, ct, val, &upload_path, server_side_processing).await {
                                    Ok((w, k, f, m)) => {
                                        file_writer = w;
                                        key = k;
                                        file_name = f;
                                        mime_type = m;
                                    },
                                    Err(_) => break 'outer
                                }
                            } else {
                                break 'outer;
                            }
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
                ParseResult::Continue(val) => {
                    parse_start += val.len();

                    match field_type {
                        FormField::Invalid => break 'outer,
                        FormField::Files => {
                            // handle files
                            let write_result = match &mut file_writer {
                                Writer::Basic(writer) => write(writer, val).await,
                                Writer::Encrypted(writer) => write(writer, val).await,
                                Writer::None => break 'outer
                            };

                            if write_result.is_err() {
                                break 'outer;
                            }
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
                        if form.parse_field(&field_type, &field_buf[..field_write_start]) {
                            upload_success = true;
                        } else {
                            break 'outer;
                        } 
                    }
                    break 'outer;
                },
                ParseResult::NeedMoreData => {
                    if parse_start == 0 {
                        // The buffer is not big enough for another read. *very*
                        // unlikely to happen for a legitimate upload and not
                        // possible to handle without allocating arbitrary
                        // ammounts of memory.
                        break 'outer;
                    } else {
                        break;
                    }
                },
                // An error
                ParseResult::Error => break 'outer
            }
        }

        // The buffer may contain incomplete data at the end, so we copy it to
        // the front of the buffer and make sure it doesn't get read over
        buf.copy_within(parse_start.., 0);
        read_start = buf.len() - parse_start;
    }

    //println!("\nUPLOAD SIZE: {}\n", total_bytes);
    println!("{:?}", form);

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

fn get_file_name(cd: &str) -> Option<&str> {
    let (_, name) = cd.split_once("filename=")?;
    let name = name.trim();
    if name.len() > 2 && name.starts_with('"') && name.ends_with('"') {
        Some(&name[1..(name.len() - 1)])
    } else {
        None
    }
}

// Return writer, key, file name, mime type
async fn handle_file_start(
    cd: &str, ct: &str, val: &[u8], upload_path: &PathBuf,
    server_side_processing: bool) -> Result<(Writer, Option<Vec<u8>>, Option<Vec<u8>>, Option<Vec<u8>>)>
{
    let file_name_str = match get_file_name(cd) {
        Some(file_name) => Ok(file_name),
        None => Err(Error::from(ErrorKind::InvalidInput))
    }?;
    let mime_type_str = ct;

    // let server_side_processing = true;

    if server_side_processing {
        let (mut writer, key, file_name, mime_type)
            = EncryptedFileWriter::new(&upload_path, file_name_str, mime_type_str)?;
        write(&mut writer, val).await?;

        Ok((Writer::Encrypted(writer), Some(key), Some(file_name), Some(mime_type)))
    } else {
        let file_name = Some(file_name_str.as_bytes().to_owned());
        let mime_type = Some(mime_type_str.as_bytes().to_owned());
        let mut writer = FileWriter::new(&upload_path)?;
        write(&mut writer, val).await?;

        Ok((Writer::Basic(writer), None, file_name, mime_type))
    }
}

// Wrapper struct for writer so we can send it into the blocking thread pool
struct WriterContainer<W>
where W: TranspoFileWriter
{
    writer: *mut W,
    bytes: *const [u8]
}
unsafe impl<W: TranspoFileWriter> Send for WriterContainer<W> {}

// This function is some crazy bullshit which allows writing to files in the
// blocking thread pool without having to copy the data to write
async fn write<W>(writer: &mut W, bytes: &[u8]) -> Result<usize>
where W: TranspoFileWriter + 'static
{
    let container = WriterContainer {
        writer: writer as *mut W,
        bytes: bytes as *const [u8]
    };

    unblock(move || {
        let container = container;
        // The async task which calls this function waits for its completion
        // before progressing, so we know that data behind these raw pointers
        // will not be invalid when we dereference.
        unsafe {
            let writer = &mut *container.writer;
            let bytes = &*container.bytes;
            writer.write(bytes)
        }
    }).await
}
