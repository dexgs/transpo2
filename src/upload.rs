use crate::multipart_form::{self, *};
use crate::b64;
use crate::files::*;
use crate::constants::*;
use crate::config::*;
use crate::db::*;
use crate::http_errors::*;
use crate::templates::*;
use crate::translations::*;
use crate::quotas::*;
use crate::cleanup::delete_upload;

use std::{cmp, fs, str};
use std::io::{Result, Error, ErrorKind};
use std::sync::Arc;
use std::path::PathBuf;
use std::net::IpAddr;
use std::time;
use rand::{thread_rng, Rng};

use trillium::Conn;
use trillium_websockets::{WebSocketConn, Message, tungstenite::protocol::frame::coding::CloseCode};
use trillium_askama::AskamaConnExt;

use tokio::io::AsyncWriteExt;

use smol::prelude::*;
use smol::io::{AsyncReadExt};

use blocking::{unblock, Unblock};

use smol_timeout::TimeoutExt;

use chrono::offset::Local;
use chrono::Duration;

use urlencoding::decode;

use argon2::{Argon2, PasswordHasher};
use argon2::password_hash::{rand_core::OsRng, SaltString};


// Make sure storage capacity is not exceeded after reading this many bytes
const STORAGE_CHECK_INTERVAL: usize = 1024 * 1024 * 10;

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


const MINUTES_QUERY: &'static str = "minutes";
const PASSWORD_QUERY: &'static str = "password";
const MAX_DOWNLOADS_QUERY: &'static str = "max-downloads";
const FILE_NAME_QUERY: &'static str = "file-name";
const MIME_TYPE_QUERY: &'static str = "mime-type";

enum UploadError {
    FileSize = 1,
    Quota = 2,
    Storage = 3,
    Protocol = 4,
    Cancelled = 5,

    Other = 0
}

impl From<Error> for UploadError {
    fn from(_: Error) -> Self {
        Self::Other
    }
}


#[derive(Default)]
struct UploadQuery {
    minutes: Option<u32>,
    max_downloads: Option<u32>,
    password: Option<String>,
    file_name: Option<Vec<u8>>,
    mime_type: Option<Vec<u8>>
}

impl UploadQuery {
    fn new(query: &str) -> Option<Self> {
        const MAX_LEN: usize = 4096;

        let mut upload_query = Self::default();

        for field in query.split('&') {
            if let Some((key, value)) = field.split_once('=') {
                if upload_query.is_key_defined(key) {
                    return None;
                }

                if value.len() > MAX_LEN {
                    return None;
                }

                match key {
                    MINUTES_QUERY => upload_query.minutes = Some(value.parse().ok()?),
                    PASSWORD_QUERY => upload_query.password = Some(decode(value).ok().map(|s| s.into_owned())?),
                    MAX_DOWNLOADS_QUERY => upload_query.max_downloads = Some(value.parse().ok()?),
                    FILE_NAME_QUERY => upload_query.file_name = Some(value.to_owned().into_bytes()),
                    MIME_TYPE_QUERY => upload_query.mime_type = Some(value.to_owned().into_bytes()),
                    _ => return None
                }
            }
        }

        Some(upload_query)
    }

    fn is_key_defined(&self, key: &str) -> bool {
        match key {
            MINUTES_QUERY => self.minutes.is_some(),
            PASSWORD_QUERY => self.password.is_some(),
            MAX_DOWNLOADS_QUERY => self.max_downloads.is_some(),
            FILE_NAME_QUERY => self.file_name.is_some(),
            MIME_TYPE_QUERY => self.mime_type.is_some(),
            _ => false
        }
    }

    fn get_values(self) -> Option<(u32, Option<u32>, Option<String>, Option<Vec<u8>>, Option<Vec<u8>>)> {
        Some((
                self.minutes?,
                self.max_downloads,
                self.password,
                self.file_name,
                self.mime_type
        ))
    }
}


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

#[derive(Default)]
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
    fn new(
        server_side_processing: bool, minutes: u32, max_downloads: Option<u32>,
        password: Option<String>) -> Self
    {
        let mut form = Self::default();
        form.server_side_processing = Some(server_side_processing);

        let days = minutes / (60 * 24);
        let hours = (minutes % (60 * 24)) / 60;
        let minutes = minutes % 60;

        form.days = Some(days as u16);
        form.hours = Some(hours as u8);
        form.minutes = Some(minutes as u8);

        if let Some(max_downloads) = max_downloads {
            form.enable_max_downloads = Some(true);
            form.max_downloads = Some(max_downloads as u32);
        }

        if let Some(password) = password {
            form.enable_password = Some(true);
            form.password = Some(password);
        }

        form
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

    fn is_password_protected(&self) -> bool {
        self.enable_password.unwrap_or(false) && self.password.is_some()
    }

    fn has_time_limit(&self) -> bool {
        self.minutes.is_some() && self.hours.is_some() && self.days.is_some()
    }
}


enum Writer {
    Basic(Unblock<FileWriter>),
    Encrypted(Unblock<EncryptedFileWriter>),
    EncryptedZip(Unblock<EncryptedZipWriter>)
}

impl Writer {
    async fn write(&mut self, buf: &[u8]) -> Result<()> {
        match self {
            Writer::Basic(writer) => {
                writer.write_all(buf).await
            },
            Writer::Encrypted(writer) => {
                writer.write_all(buf).await
            },
            Writer::EncryptedZip(writer) => {
                writer.write_all(buf).await
            }
        }
    }

    async fn flush(&mut self) -> Result<()> {
        match self {
            Writer::Basic(writer) => writer.flush().await,
            Writer::Encrypted(writer) => writer.flush().await,
            Writer::EncryptedZip(writer) => writer.flush().await
        }
    }
}

fn create_upload_storage_dir(storage_path: PathBuf) -> (i64, String, PathBuf) {
    // Note: we check the filesystem to avoid duplicate upload IDs.
    let mut rng = thread_rng();
    loop {
        let id = rng.gen();
        let id_string = String::from_utf8(b64::i64_to_b64_bytes(id)).unwrap();

        let dir = storage_path.join(&id_string);
        // This will fail if the directory already exists
        if fs::create_dir(&dir).is_ok() {
            return (id, id_string, dir);
        }
    }
}

pub async fn handle_websocket(
    mut conn: WebSocketConn, config: Arc<TranspoConfig>,
    db_backend: DbBackend, quotas_data: Option<(Quotas, IpAddr)>) -> Result<()>
{
    let query = UploadQuery::new(conn.querystring());

    if let Some((minutes, max_downloads, password, file_name, mime_type)) =
        query.and_then(|q| q.get_values())
    {
        let (upload_id, upload_id_string, upload_dir) = {
            let storage_path = config.storage_dir.clone();
            unblock(|| create_upload_storage_dir(storage_path))
        }.await;

        let upload_path = upload_dir.join("upload");

        let form = UploadForm::new(true, minutes, max_downloads, password);

        let db_write_succeeded = write_to_db(
            form, upload_id, file_name, mime_type,
            db_backend, config.clone()).await.is_some();

        if db_write_succeeded {
            conn.send_string(upload_id_string.clone()).await;

            let upload_result = websocket_read_loop(
                &mut conn, &upload_path, config.clone(), quotas_data).await;

            match upload_result {
                Ok(()) => {
                    let write_is_completed_success =
                        write_is_completed(upload_id, db_backend, config.clone()).await.is_some();

                    if write_is_completed_success {
                        // Don't handle error, since client may have already closed its
                        // end in which case closing here will return an error, but
                        // this error should *not* cause the upload to fail.
                        drop(conn.send(Message::Close(None)).await);
                        return Ok(()); // return early
                    } else {
                        drop(conn.send(Message::Binary(vec![UploadError::Other as u8])).await);
                    }
                },
                Err(e) => {
                    drop(conn.send(Message::Binary(vec![e as u8])).await);
                }
            }
        }


        unblock(move || {
            let db_connection = establish_connection(db_backend, &config.db_url);
            delete_upload(upload_id, &config.storage_dir, &db_connection);
        }).await;
    }

    drop(conn.send(Message::Close(None)).await);
    Err(Error::new(ErrorKind::Other, "Upload failed"))
}

async fn websocket_read_loop(
    conn: &mut WebSocketConn, upload_path: &PathBuf, config: Arc<TranspoConfig>,
    quotas_data: Option<(Quotas, IpAddr)>) -> std::result::Result<(), UploadError>
{
    if is_storage_full(config.clone()).await? {
        return Err(UploadError::Storage);
    }

    let timeout_duration = time::Duration::from_millis(config.read_timeout_milliseconds as u64);
    let mut writer = AsyncFileWriter::new(
        &upload_path, config.max_upload_size_bytes).await?;
    let mut bytes_read_interval = 0;

    while let Some(Ok(msg)) = conn
        .next()
        .timeout(timeout_duration).await
        .flatten()
    {
        match msg {
            Message::Binary(b) => {
                if let Some(true) = quotas_data.as_ref().map(
                    |(q, a)| q.exceeds_quota(a, b.len()))
                {
                    return Err(UploadError::Quota);
                } else if b.len() > FORM_READ_BUFFER_SIZE * 2 {
                    return Err(UploadError::Protocol);
                } else {
                    bytes_read_interval += b.len();
                    if bytes_read_interval > STORAGE_CHECK_INTERVAL {
                        bytes_read_interval = 0;

                        if is_storage_full(config.clone()).await? {
                            return Err(UploadError::Storage);
                        }

                        if !upload_path.exists() {
                            return Err(UploadError::Other);
                        }
                    }

                    if let Err(e) = writer.write_all(&b).await {
                        return match e.kind() {
                            ErrorKind::WriteZero => Err(UploadError::FileSize),
                            _ => Err(UploadError::Other)
                        };
                    }
                }
            },
            Message::Close(Some(closeframe)) => {
                if closeframe.code == CloseCode::Normal {
                    writer.flush().await?;
                    return Ok(());
                } else {
                    return Err(UploadError::Cancelled);
                }
            },
            _ => {
                drop(conn.close().await);
                return Err(UploadError::Protocol);
            }
        }
    }

    // websocket not properly closed
    Err(UploadError::Protocol)
}

pub async fn handle_post(
    mut conn: Conn, config: Arc<TranspoConfig>, translation: Translation,
    db_backend: DbBackend, quotas_data: Option<(Quotas, IpAddr)>) -> Conn
{
    // Get the boundary of the multi-part form
    let boundary = match get_boundary(&conn) {
        Some(boundary) => boundary,
        None => return error_400(conn, config, translation)
    };
    let boundary = format!("\r\n--{}", boundary);
    if boundary.len() > MAX_FORM_BOUNDARY_LENGTH
    || !boundary.starts_with(EXPECTED_BOUNDARY_START)
    {
        // This is unlikely to happen unless someone is trying to abuse the
        // slowest path in the parser: a long boundary that contains every
        // possible byte value.
        return error_400(conn, config, translation);
    }

    let (upload_id, upload_id_string, upload_dir) = {
        let storage_path = config.storage_dir.clone();
        unblock(|| create_upload_storage_dir(storage_path))
    }.await;

    let upload_path = upload_dir.join("upload");

    let mut file_writer: Option<Writer> = None;
    let mut key: Option<Vec<u8>> = None;

    let query = UploadQuery::new(conn.querystring());

    let (mut form, mut file_name, mut mime_type) = if let Some(
        (minutes, max_downloads, password, file_name, mime_type))
        = query.and_then(|q| q.get_values())
    {
        (UploadForm::new(true, minutes, max_downloads, password), file_name, mime_type)
    } else {
        (UploadForm::default(), None, None)
    };

    let mut db_write_success = false;

    // If a time limit has already been provided via the query string, write
    // the current data in the form to the DB to allow the file to be downloaded
    // while it uploads. If the client did not include the needed information
    // in the query string, it must provide it in the form body which will be
    // read by `parse_upload_form`.
    if form.has_time_limit() {
        db_write_success = write_to_db(
            form, upload_id, file_name, mime_type,
            db_backend, config.clone()).await.is_some();
        file_name = None;
        mime_type = None;
        form = UploadForm::default();
    }

    let req_body = conn.request_body().await;
    let parse_result = parse_upload_form(
        req_body, boundary, &upload_path, &mut form, &mut file_writer, &mut key,
        &mut file_name, &mut mime_type, config.clone(), quotas_data).await;
    let parse_success = match parse_result {
        Ok(result) => result,
        Err(_) => false
    };

    let is_password_protected = form.is_password_protected();

    // If a DB entry has not yet been written for the upload, and parsing the
    // upload body succeeded, try to write one now.
    if parse_success && !db_write_success {
        db_write_success = write_to_db(
            form, upload_id, file_name, mime_type,
            db_backend, config.clone()).await.is_some();
    }

    // write that the upload is completed into the db
    let write_is_completed_success =
        write_is_completed(upload_id, db_backend, config.clone()).await.is_some();

    let upload_success =
        parse_success
        && db_write_success
        && write_is_completed_success;

    // Respond to the client
    if upload_success {
        if let Some(key) = key {
            // If the server handled encryption + archiving
            let key_string = String::from_utf8(key).unwrap();
            if conn.headers().has_header("User-Agent") {
                // If the client is probably a browser
                let upload_url = if is_password_protected {
                    format!("{}#{}", upload_id_string, key_string)
                } else {
                    format!("{}?nopass#{}", upload_id_string, key_string)
                };

                let template = UploadLinkTemplate {
                    app_name: config.app_name.clone(),
                    upload_url: upload_url,
                    upload_id: upload_id_string,
                    t: translation
                };
                conn.render(template).halt()
            } else {
                // If the client is probably a tool like curl
                conn
                    .with_status(200)
                    .with_header("Content-Type", "application/json")
                    .with_body(format!("\"{}#{}\"", upload_id_string, key_string))
                    .halt()
            }
        } else {
            // If the client handled encryption + archiving
            conn
                .with_status(200)
                .with_header("Content-Type", "application/json")
                .with_body(format!("\"{}\"", upload_id_string))
                .halt()
        }
    } else {
        let response = error_400(conn, config.clone(), translation);

        unblock(move || {
            let db_connection = establish_connection(db_backend, &config.db_url);
            delete_upload(upload_id, &config.storage_dir, &db_connection);
        }).await;

        response
    }
}

async fn is_storage_full(config: Arc<TranspoConfig>) -> Result<bool> {
    unblock(move || {
        Ok(get_storage_size(&config.storage_dir)? > config.max_storage_size_bytes)
    }).await
}

async fn parse_upload_form<R>(
    mut req_body: R, boundary: String, upload_path: &PathBuf,
    form: &mut UploadForm, file_writer: &mut Option<Writer>,
    key: &mut Option<Vec<u8>>, file_name: &mut Option<Vec<u8>>,
    mime_type: &mut Option<Vec<u8>>, config: Arc<TranspoConfig>,
    quotas_data: Option<(Quotas, IpAddr)>) -> Result<bool>
where R: AsyncReadExt + Unpin
{
    if is_storage_full(config.clone()).await? {
        return Err(Error::new(ErrorKind::Other, "Storage capacity exceeded"));
    }

    let timeout_duration = time::Duration::from_millis(
        config.read_timeout_milliseconds as u64);
    let mut upload_success = false;
    let mut buf = [0; FORM_READ_BUFFER_SIZE];
    let boundary_byte_map = byte_map(boundary.as_bytes());
    // Make the first boundary start with a newline to simplify parsing
    (&mut buf[..2]).copy_from_slice(b"\r\n");
    let mut read_start = 2;

    let mut field_type = FormField::Invalid;
    // Form fields other than files are expected to fit in this buffer.
    // If they do not, error 400 will be returned.
    let mut field_buf = [0; FORM_FIELD_BUFFER_SIZE];
    let mut field_write_start = 0;

    let mut bytes_read_interval = 0;

    'outer: while let Some(Ok(bytes_read)) = req_body
        .read(&mut buf[read_start..])
        .timeout(timeout_duration).await
    {
        if bytes_read == 0 {
            break 'outer;
        }

        if let Some(true) = quotas_data.as_ref().map(
            |(q, a)| q.exceeds_quota(a, bytes_read))
        {
            return Err(Error::new(ErrorKind::Other, "Quota exceeded"));
        }

        bytes_read_interval += bytes_read;
        if bytes_read_interval > STORAGE_CHECK_INTERVAL {
            bytes_read_interval = 0;
            if is_storage_full(config.clone()).await? {
                return Err(Error::new(ErrorKind::Other, "Storage capacity exceeded"));
            }
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
                                                    config.max_upload_size_bytes,
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
                    if field_type != FormField::Invalid {
                        // parse the value of the previous field, if it wasn't
                        // the contents of the upload
                        if field_type != FormField::Files {
                            upload_success = form.parse_field(&field_type, &field_buf[..field_write_start]);
                        } else {
                            upload_success = true;
                        }

                        if let Some(mut writer) = file_writer.take() {
                            upload_success = writer.flush().await.is_ok() && upload_success;

                            match writer {
                                Writer::EncryptedZip(writer) => {
                                    // Finish the Zip archive by writing the
                                    // end of central directory record
                                    let mut inner_writer = writer.into_inner().await;
                                    unblock::<Result<()>, _>(move || {
                                        inner_writer.finish_file()?;
                                        inner_writer.finish()?;
                                        Ok(())
                                    }).await?;
                                },
                                Writer::Encrypted(mut writer) => {
                                    writer.with_mut(|w| w.finish()).await?;
                                    writer.flush().await?;
                                },
                                _ => {}
                            }
                        }

                        if is_storage_full(config.clone()).await? {
                            return Err(Error::new(ErrorKind::Other, "Storage capacity exceeded"));
                        }
                    }

                    break 'outer;
                },
                ParseResult::NeedMoreData => {
                    if parse_start == 0 && buf.len() == FORM_READ_BUFFER_SIZE {
                        // The buffer is not big enough for another read without
                        // discarding any data. This is *very* unlikely to
                        // happen for a legitimate upload and not possible to
                        // handle without allocating arbitrary amounts of
                        // memory.
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
        if parse_start != 0 {
            buf.copy_within(parse_start.., 0);
        }
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
    max_upload_size: usize,
    compression_level: usize) -> Result<(Option<Vec<u8>>, Option<Vec<u8>>, Option<Vec<u8>>)>
{
    let file_name_str = match get_file_name(cd) {
        Some(file_name) => Ok(file_name),
        None => Err(Error::from(ErrorKind::InvalidInput))
    }?;

    let mime_type_str = ct;
    // https://datatracker.ietf.org/doc/html/rfc4288#section-4.2
    if mime_type_str.len() > 255 {
        return Err(Error::new(ErrorKind::InvalidInput, "Mime type is too long"));
    } else if mime_type_str.is_empty() {
        return Err(Error::new(ErrorKind::InvalidInput, "Mime type is empty"));
    }

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
                            &upload_path, max_upload_size,
                            compression_level as u8)?;
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
                            &upload_path, max_upload_size,
                            file_name_str, mime_type_str)?;
                    let inner_writer = Unblock::with_capacity(FORM_READ_BUFFER_SIZE, inner_writer);

                    *file_writer = Some(Writer::Encrypted(inner_writer));
                    return Ok((Some(key), Some(file_name), Some(mime_type)));
                }
            } else {
                // Single file upload with client-side processing
                let file_name = Some(file_name_str.as_bytes().to_owned());
                let mime_type = Some(mime_type_str.as_bytes().to_owned());
                let inner_writer = FileWriter::new(&upload_path, max_upload_size)?;
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
    db_backend: DbBackend, config: Arc<TranspoConfig>) -> Option<usize>
{

    let time_limit_minutes = 
        (form.minutes? as usize)
        + (form.hours? as usize) * 60
        + (form.days? as usize) * 60 * 24;
    let time_limit_minutes = cmp::min(time_limit_minutes, config.max_upload_age_minutes);

    let file_name = String::from_utf8(file_name?).ok()?;
    let mime_type = String::from_utf8(mime_type?).ok()?;

    let password_hash = if form.is_password_protected() {
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

    let expire_after = Local::now().naive_utc()
        + Duration::minutes(time_limit_minutes as i64);

    let upload = Upload {
        id: id,
        file_name: file_name,
        mime_type: mime_type,
        password_hash: password_hash,
        remaining_downloads: remaining_downloads,
        expire_after: expire_after,
        is_completed: false
    };

    unblock(move || {
        let db_connection = establish_connection(db_backend, &config.db_url);
        let num_modified_rows = upload.insert(&db_connection)?;

        Some(num_modified_rows)
    }).await
}

async fn write_is_completed(
    id: i64, db_backend: DbBackend, config: Arc<TranspoConfig>) -> Option<usize>
{
    unblock(move || {
        let db_connection = establish_connection(db_backend, &config.db_url);
        let num_modified_rows = Upload::set_is_completed(id, true, &db_connection)?;

        Some(num_modified_rows)
    }).await
}
