// use crate::concurrency::Accessors;
use crate::multipart_form::{self, *};
// use crate::random_bytes::*;
use crate::b64;
use crate::files::*;
use crate::constants::*;
use crate::config::*;
use crate::db::*;
use crate::http_errors::*;
use crate::templates::*;
use std::{cmp, fs, str};
use std::io::{Result, Error, ErrorKind};
use std::sync::Arc;
use std::path::PathBuf;
use trillium::Conn;
use trillium_http::ReceivedBody;
use trillium_http::transport::BoxedTransport;
use trillium_askama::AskamaConnExt;
use smol::prelude::*;
use blocking::{unblock, Unblock};
use chrono::offset::Local;
use chrono::Duration;
use argon2::{Argon2, PasswordHasher};
use argon2::password_hash::{rand_core::OsRng, SaltString};
use rand::{thread_rng, Rng};


const EXPECTED_BOUNDARY_START: &'static str = "\r\n-----------------------";

// Content-Disposition for valid form fields
const SERVER_SIDE_PROCESSING_CD: &'static str = "form-data; name=\"server-side-processing\"";
const ENABLE_MULTIPLE_FILES_CD: &'static str = "form-data; name=\"enable-multiple-files\"";
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
    EnableMultipleFiles,
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
            ENABLE_MULTIPLE_FILES_CD => FormField::EnableMultipleFiles,
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
    enable_multiple_files: Option<bool>,
    days: Option<u16>,
    hours: Option<u8>,
    minutes: Option<u8>,
    enable_max_downloads: Option<bool>,
    max_downloads: Option<u32>,
    enable_password: Option<bool>,
    password: Option<String>
}

impl UploadForm {
    fn new() -> Self {
        Self {
            server_side_processing: None,
            enable_multiple_files: None,
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
            FormField::EnableMultipleFiles => self.enable_multiple_files.is_none(),
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
                    FormField::EnableMultipleFiles => Self::parse_bool_value(value, &mut self.enable_multiple_files),
                    FormField::Days => Self::parse_from_str(value, &mut self.days),
                    FormField::Hours => Self::parse_from_str(value, &mut self.hours),
                    FormField::Minutes => Self::parse_from_str(value, &mut self.minutes),
                    FormField::EnableMaxDownloads => Self::parse_bool_value(value, &mut self.enable_max_downloads),
                    FormField::MaxDownloads => Self::parse_from_str(value, &mut self.max_downloads),
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

    fn parse_from_str<I>(value: &str, field: &mut Option<I>) -> bool
    where I: str::FromStr
    {
        match *field {
            Some(_) => false,
            None => match value.parse::<I>() {
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


enum Writer {
    Basic(Unblock<FileWriter>),
    Encrypted(Unblock<EncryptedFileWriter>),
    EncryptedZip(Unblock<EncryptedZipWriter>)
    // None
}

impl Writer {
    /*
    fn is_none(&self) -> bool {
        match self {
            Writer::None => true,
            _ => false
        }
    }
    */

    async fn write(&mut self, buf: &[u8]) -> Result<usize> {
        match self {
            Writer::Basic(writer) => {
                writer.flush().await?;
                writer.write(buf).await
            },
            Writer::Encrypted(writer) => {
                writer.flush().await?;
                writer.write(buf).await
            },
            Writer::EncryptedZip(writer) => {
                writer.flush().await?;
                writer.write(buf).await
            }
            // Writer::None => Err(Error::from(ErrorKind::Other))
        }
    }

    async fn flush(&mut self) -> Result<()> {
        match self {
            Writer::Basic(writer) => writer.flush().await,
            Writer::Encrypted(writer) => writer.flush().await,
            Writer::EncryptedZip(writer) => writer.flush().await
            // Writer::None => Err(Error::from(ErrorKind::Other))
        }
    }
}


pub async fn handle(
    mut conn: Conn, config: Arc<TranspoConfig>,
    // accessors: Accessors, db_backend: DbBackend) -> Conn
    db_backend: DbBackend) -> Conn
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
    // let boundary_byte_map = byte_map(boundary.as_bytes());

    let (upload_id, upload_id_string, upload_dir) = {
        // let accessors = accessors.clone();
        let storage_path = config.storage_dir.clone();
        unblock(move || {
            let mut rng = thread_rng();
            loop {
                let id = rng.gen();
                let id_string = String::from_utf8(b64::i64_to_b64_bytes(id)).unwrap();

                // if accessors.access(id, true).is_some() {
                let dir = storage_path.join(&id_string);
                // This will fail if the directory already exists
                if fs::create_dir(&dir).is_ok() {
                    return (id, id_string, dir);
                }
                // }
            }
        })
    }.await;
    let upload_path = upload_dir.join("upload");

    let mut file_writer: Option<Writer> = None;
    let mut key: Option<Vec<u8>> = None;
    let mut file_name: Option<Vec<u8>> = None;
    let mut mime_type: Option<Vec<u8>> = None;

    // Form fields
    let mut form = UploadForm::new();

    let req_body = conn.request_body().await;

    let parse_result = parse_upload_request(
        req_body, boundary, &upload_path, db_backend, &mut form,
        &mut file_writer, &mut key, &mut file_name, &mut mime_type,
        config.clone()).await;

    let upload_success = match parse_result {
        Ok(result) => result,
        Err(_) => false
    };

    //println!("{:?}", form);

    // Respond to the client
    if upload_success
    && write_to_db(form, upload_id.clone(), file_name, mime_type, db_backend,
    config.clone()).await.is_some()
    {
        if let Some(key) = key {
            let key_string = String::from_utf8(key).unwrap();
            if conn.headers().has_header("User-Agent") {
                // If the client is probably a browser
                let template = UploadLinkTemplate {
                    app_name: config.app_name.clone(),
                    upload_url: format!("{}#{}", upload_id_string, key_string),
                    upload_id: upload_id_string,
                };
                conn.render(template).halt()
            } else {
                // If the client is probably a tool like curl
                conn
                    .with_status(200)
                    .with_header("Content-Type", "application/json")
                    .with_body(format!("\"{}#{}\"", upload_id, key_string))
                    .halt()
            }
        } else {
            conn
                .with_status(200)
                .with_header("Content-Type", "application/json")
                .with_body(format!("\"{}\"", upload_id))
                .halt()
        }
    } else {
        unblock(move || {
            if upload_dir.exists() {
                std::fs::remove_dir_all(upload_dir)
                .expect("Deleting incomplete upload");
            }
        }).await;

        error_400(conn)
    }
}

async fn parse_upload_request(
    mut req_body: ReceivedBody<'_, BoxedTransport>, boundary: String, upload_path: &PathBuf,
    db_backend: DbBackend, form: &mut UploadForm,
    file_writer: &mut Option<Writer>, key: &mut Option<Vec<u8>>,
    file_name: &mut Option<Vec<u8>>, mime_type: &mut Option<Vec<u8>>,
    config: Arc<TranspoConfig>) -> Result<bool>
{
    let mut upload_success = false;
    let mut buf = [0; FORM_READ_BUFFER_SIZE];
    let boundary_byte_map = byte_map(boundary.as_bytes());
    // Make the first boundary start with a newline to simplify parsing
    (&mut buf[..2]).copy_from_slice(b"\r\n");
    let mut read_start = 2;

    let mut field_type = FormField::Invalid;
    // Form fields other than files are expected to fit in this buffer. If they
    // do not, error 400 will be returned.
    let mut field_buf = [0; FORM_FIELD_BUFFER_SIZE];
    let mut field_write_start = 0;

    'outer: while let Ok(bytes_read) = req_body.read(&mut buf[read_start..]).await {
        if bytes_read == 0 {
            break 'outer;
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
                            return Err(Error::new(
                                    ErrorKind::InvalidData,
                                    "Error parsing form field"));
                        }
                    }

                    // handle the new field
                    let new_field_type = match_content_disposition(cd);
                    match new_field_type {
                        FormField::Invalid => {
                            return Err(Error::new(
                                    ErrorKind::InvalidData,
                                    "Error invalid form field type"));
                        },
                        FormField::Files => {
                            let server_side_processing = match form.server_side_processing {
                                None | Some(false) => false,
                                Some(true) => true
                            };

                            let enable_multiple_files = match form.enable_multiple_files {
                                None | Some(false) => false,
                                Some(true) => true
                            };

                            let is_first_file = file_writer.is_none();

                            match handle_file_start(cd, ct, &upload_path, file_writer,
                                                    server_side_processing,
                                                    enable_multiple_files,
                                                    config.max_storage_size_bytes,
                                                    config.max_upload_size_bytes,
                                                    db_backend,
                                                    &config.db_url,
                                                    config.compression_level).await
                            {
                                Ok((k, f, m)) => {
                                    if is_first_file {
                                        *key = k;
                                        *file_name = f;
                                        *mime_type = m;
                                    }
                                },
                                Err(_) => {
                                    return Err(Error::new(
                                            ErrorKind::InvalidData,
                                            "File upload started when not allowed"));
                                }
                            }

                            match file_writer {
                                Some(writer) => {
                                    writer.write(val).await?;
                                },
                                None => {
                                    return Err(Error::new(
                                            ErrorKind::InvalidData,
                                            "Cannot write file contents without writer"));
                                }
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
                                return Err(Error::new(
                                        ErrorKind::InvalidData,
                                        "Invalid form field contents"));
                            }
                        }
                    }

                    field_type = new_field_type;
                },
                // The continuation of the value of the previous field
                ParseResult::Continue(val) => {
                    parse_start += val.len();

                    match field_type {
                        FormField::Invalid => {
                            return Err(Error::new(
                                    ErrorKind::InvalidData,
                                    "Error invalid form field type"));
                        },
                        FormField::Files => match file_writer {
                            Some(writer) => {
                                writer.write(val).await?;
                            },
                            None => {
                                return Err(Error::new(
                                        ErrorKind::InvalidData,
                                        "Cannot write file contents without writer"));
                            }
                        },
                        _ => {
                            if field_write_start + val.len() <= field_buf.len() {
                                // copy new data into the field buffer
                                (&mut field_buf[field_write_start..][..val.len()])
                                    .copy_from_slice(val);
                                field_write_start += val.len();
                            } else {
                                return Err(Error::new(
                                        ErrorKind::Other,
                                        "Form field is too big"));
                            }
                        }
                    }
                },
                // The end of the form
                ParseResult::Finished => {
                    if field_type != FormField::Invalid { // && file_writer.flush().await.is_ok() {
                        // parse the value of the previous field, if it wasn't
                        // the contents of the upload
                        if field_type != FormField::Files {
                            upload_success = form.parse_field(&field_type, &field_buf[..field_write_start]);
                        }

                        if let Some(mut writer) = file_writer.take() {
                            upload_success = writer.flush().await.is_ok();

                            if let Writer::EncryptedZip(writer) = writer {
                                let mut inner_writer = writer.into_inner().await;
                                upload_success = unblock(move || {
                                    inner_writer.finish_file().is_ok()
                                        && inner_writer.finish().is_ok()
                                }).await && upload_success;
                            }
                        }
                    }

                    break 'outer;
                },
                ParseResult::NeedMoreData => {
                    if parse_start == 0 {
                        // The buffer is not big enough for another read. *very*
                        // unlikely to happen for a legitimate upload and not
                        // possible to handle without allocating arbitrary
                        // amounts of memory.
                        return Err(Error::new(
                                ErrorKind::Other,
                                "Form field is too big"));
                    } else {
                        break;
                    }
                },
                // An error
                ParseResult::Error => {
                    return Err(Error::new(
                            ErrorKind::Other,
                            "Parse error"));
                }
            }
        }

        // The buffer may contain incomplete data at the end, so we copy it to
        // the front of the buffer and make sure it doesn't get read over
        buf.copy_within(parse_start.., 0);
        read_start = buf.len() - parse_start;
    }

    Ok(upload_success)
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
    cd: &str, ct: &str, upload_path: &PathBuf, file_writer: &mut Option<Writer>,
    server_side_processing: bool,
    enable_multiple_files: bool,
    max_storage_size: usize,
    max_upload_size: usize,
    db_backend: DbBackend,
    db_url: &str,
    compression_level: usize) -> Result<(Option<Vec<u8>>, Option<Vec<u8>>, Option<Vec<u8>>)>
{
    let db_connection = establish_connection(db_backend, db_url);

    let file_name_str = match get_file_name(cd) {
        Some(file_name) => Ok(file_name),
        None => Err(Error::from(ErrorKind::InvalidInput))
    }?;

    let mime_type_str = ct;
    // https://datatracker.ietf.org/doc/html/rfc4288#section-4.2
    if mime_type_str.len() > 255 {
        return Err(Error::from(ErrorKind::InvalidInput));
    }

    // let server_side_processing = true;
    // println!("{enable_multiple_files}");

    match file_writer {
        Some(writer) => {
            if let Writer::EncryptedZip(writer) = writer {
                // New file for existing multi-file upload
                let file_name_str = file_name_str.to_owned();

                writer.with_mut::<Result<()>, _>(move |writer| {
                    writer.finish_file()?;
                    writer.start_new_file(&file_name_str)?;
                    Ok(())
                }).await?;

                return Ok((None, None, None));
            }
        },
        None => {
            if server_side_processing {
                if enable_multiple_files {
                    // Multi-file upload with server-side processing on
                    let (mut inner_writer, key, file_name, mime_type)
                        = EncryptedZipWriter::new(
                            &upload_path, max_storage_size, max_upload_size,
                            db_connection, compression_level as u8)?;
                    let file_name_str = file_name_str.to_owned();

                    let inner_writer = unblock::<Result<Unblock<EncryptedZipWriter>>, _>(move || {
                        inner_writer.start_new_file(&file_name_str)?;
                        Ok(Unblock::with_capacity(FORM_READ_BUFFER_SIZE, inner_writer))
                    }).await;

                    *file_writer = Some(Writer::EncryptedZip(inner_writer?));
                    return Ok((Some(key), Some(file_name), Some(mime_type)));
                } else {
                    // Single file upload with server-side processing on
                    let (inner_writer, key, file_name, mime_type)
                        = EncryptedFileWriter::new(
                            &upload_path, max_storage_size, max_upload_size,
                            db_connection, file_name_str, mime_type_str)?;
                    let inner_writer = Unblock::with_capacity(FORM_READ_BUFFER_SIZE, inner_writer);

                    *file_writer = Some(Writer::Encrypted(inner_writer));
                    return Ok((Some(key), Some(file_name), Some(mime_type)));
                }
            } else {
                // Single file upload with client-side processing
                let file_name = Some(file_name_str.as_bytes().to_owned());
                let mime_type = Some(mime_type_str.as_bytes().to_owned());
                let inner_writer = FileWriter::new(
                    &upload_path, max_storage_size, max_upload_size,
                    db_connection)?;
                let inner_writer = Unblock::with_capacity(FORM_READ_BUFFER_SIZE, inner_writer);

                *file_writer = Some(Writer::Basic(inner_writer));
                return Ok((None, file_name, mime_type));
            }
        }
    }

    Err(Error::from(ErrorKind::Other))
}


// Insert the metadata for an upload into the database. Return the number of
// affected rows (or None if there was an error)
async fn write_to_db(
    form: UploadForm, id: i64, file_name: Option<Vec<u8>>, mime_type: Option<Vec<u8>>,
    // db_backend: DbBackend, config: Arc<TranspoConfig>, accessors: Accessors) -> Option<usize>
    db_backend: DbBackend, config: Arc<TranspoConfig>) -> Option<usize>
{

    let time_limit_minutes = 
        (form.minutes? as usize)
        + (form.hours? as usize) * 60
        + (form.days? as usize) * 60 * 24;
    let time_limit_minutes = cmp::min(time_limit_minutes, config.max_upload_age_minutes);

    let file_name = String::from_utf8(file_name?).ok()?;
    let mime_type = String::from_utf8(mime_type?).ok()?;

    let password_protected = form.enable_password.unwrap_or(false);
    let password_hash = if password_protected {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let hash = argon2.hash_password(form.password?.as_bytes(), &salt).ok()?
            .to_string()
            .into_bytes();
        assert_eq!(hash.len(), 96);
        Some(hash)
    } else {
        None
    };

    let has_download_limit = form.enable_max_downloads.or(Some(false))?;
    let remaining_downloads = if has_download_limit {
        Some(cmp::min(form.max_downloads?, i32::MAX as u32) as i32)
    } else {
        None
    };

    let expire_after = Local::now().naive_local()
        + Duration::minutes(time_limit_minutes as i64);

    let upload = Upload {
        id: id.clone(),
        file_name: file_name,
        mime_type: mime_type,
        password_hash: password_hash,
        remaining_downloads: remaining_downloads,
        num_accessors: 0,
        expire_after: expire_after
    };

    unblock(move || {
        // if let Some(accessor) = accessors.access(id, true) {
        let db_connection = establish_connection(db_backend, &config.db_url);
        let num_modified_rows = upload.insert(&db_connection)?;
        // drop(accessor);

        Some(num_modified_rows)
        /* } else {
            None
        }*/
    }).await
}
