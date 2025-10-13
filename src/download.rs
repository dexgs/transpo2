use crate::concurrency::*;
use crate::db::*;
use crate::b64::*;
use crate::constants::*;
use crate::config::*;
use crate::files::*;
use crate::http_errors::*;
use crate::translations::*;
use crate::cleanup::delete_upload;

use std::io::{Read, Result};
use std::sync::Arc;

use blocking::*;
use trillium::{Conn, Body};

use urlencoding::{decode, encode};

use argon2::{Argon2, PasswordHash, PasswordVerifier};


struct Reader<R>
where R: Read {
    reader: R,
    accessor_mutex: AccessorMutex,
    db_backend: DbBackend,
    config: Arc<TranspoConfig>
}

impl<R> Reader<R>
where R: Read
{
    fn cleanup(&mut self) {
        let accessor = self.accessor_mutex.lock();

        // If we're the last accessor, then it's our responsibility to
        // clean up the upload if it is now invalid!
        if accessor.is_only_accessor() {
            let db_connection = establish_connection(self.db_backend, &self.config.db_url);

            let should_delete = match Upload::select_with_id(accessor.id, &db_connection) {
                Some(upload) => upload.is_expired(),
                None => true
            };

            if should_delete {
                delete_upload(accessor.id, &self.config.storage_dir, &db_connection);
            }
        }
    }
}

impl<R> Drop for Reader<R> 
where R: Read
{
    fn drop(&mut self) {
        self.cleanup();
    }
}

impl<R> Read for Reader<R>
where R: Read {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.reader.read(buf)
    }
}

#[derive(Default)]
struct DownloadQuery {
    crypto_key: Option<Vec<u8>>,
    password: Option<Vec<u8>>,
    start_index: u64
}

fn parse_query(query: &str) -> DownloadQuery {
    let mut parsed = DownloadQuery::default();

    for field in query.split('&') {
        if let Some((key, value)) = field.split_once('=') {
            match key {
                "key" => {
                    if value.len() == base64_encode_length(256 / 8) {
                        parsed.crypto_key = Some(value.to_owned().into_bytes())
                    }
                },
                "password" => parsed.password = decode(value)
                    .ok()
                    .and_then(|s| Some(s.into_owned().into_bytes())),
                "start_index" => if let Ok(start_index) = value.parse() {
                    parsed.start_index = start_index;
                }
                _ => {}
            }
        }
    }

    parsed
}

fn get_upload(
    id: i64, config: &TranspoConfig, accessors: &Accessors,
    db_connection: &DbConnection) -> Option<Upload>
{
    let accessor_mutex = accessors.access(id);
    let accessor = accessor_mutex.lock();

    let row = Upload::select_with_id(id, &db_connection)?;

    // If the row is expired and we are the only accessor, clean it up!
    let upload = if row.is_expired() {
        if accessor.is_only_accessor() {
            delete_upload(accessor.id, &config.storage_dir, &db_connection);
        }
        None
    } else {
        Some(row)
    };

    upload
}

fn check_password(password: &Option<Vec<u8>>, upload: &Upload) -> bool {
    let hash_string = upload.password_hash.as_ref()
        .map(|h| String::from_utf8_lossy(h).to_string());

    match hash_string {
        Some(hash_string) => {
            let hash_and_password = PasswordHash::new(&hash_string).ok()
                .zip(password.as_ref());

            if let Some((hash, password)) = hash_and_password {
                Argon2::default().verify_password(password, &hash).is_ok()
            } else {
                false
            }
        },
        None => true
    }
}


pub async fn info(
    conn: Conn, id_string: String, config: Arc<TranspoConfig>,
    accessors: Accessors, translation: Translation, db_backend: DbBackend) -> Conn
{
    if id_string.len() != base64_encode_length(ID_LENGTH) {
        return error_404(conn, config, translation);
    }

    let id = i64_from_b64_bytes(id_string.as_bytes()).unwrap();

    let query = parse_query(conn.querystring());
    let password = query.password;

    let config_ = config.clone();
    let info = unblock(move || {
        let db_connection = establish_connection(db_backend, &config_.db_url);
        let upload = get_upload(id, &config_, &accessors, &db_connection)?;
        let upload_path = config_.storage_dir.join(&id_string).join("upload");
        let ciphertext_size = if upload.is_completed {
            get_file_size(&upload_path).ok()?
        } else {
            0
        };

        if !check_password(&password, &upload) {
            None
        } else {
            Some((upload.file_name, upload.mime_type, ciphertext_size))
        }
    }).await;

    match info {
        Some((file_name, mime_type, file_size)) => {
            conn
                .with_status(200)
                .with_header("Content-Type", "application/json")
                .with_body(format!("{{ \
                        \"name\": \"{}\", \
                        \"mime\": \"{}\", \
                        \"size\": {} \
                    }}",
                    file_name, mime_type, file_size))
                .halt()
        },
        None => {
            error_400(conn, config, translation)
        }
    }
}


pub async fn handle(
    conn: Conn, id_string: String, config: Arc<TranspoConfig>,
    accessors: Accessors, translation: Translation, db_backend: DbBackend) -> Conn
{
    if id_string.len() != base64_encode_length(ID_LENGTH) {
        return error_404(conn, config, translation);
    }

    let id = i64_from_b64_bytes(id_string.as_bytes()).unwrap();

    let query = parse_query(conn.querystring());
    let crypto_key = query.crypto_key;
    let password = query.password;
    let start_index = query.start_index;

    let response = {
        let config = config.clone();
        unblock(move || {
            let db_connection = establish_connection(db_backend, &config.db_url);

            let upload = get_upload(id, &config, &accessors, &db_connection)?;

            // validate password
            if !check_password(&password, &upload) {
                return None;
            }

            let accessor_mutex = accessors.access(id);
            Upload::decrement_remaining_downloads(id, &db_connection)?;

            let upload_path = config.storage_dir.join(&id_string).join("upload");
            let ciphertext_size = get_file_size(&upload_path).ok()?;

            let (body, file_name, mime_type) = match crypto_key {
                // server-side decryption
                Some(key) => {
                    let (reader, mut file_name, mime_type) =
                        EncryptedFileReader::new(
                            &upload_path, start_index, upload.expire_after, upload.is_completed,
                            &key, upload.file_name.as_bytes(), upload.mime_type.as_bytes()).ok()?;

                    // If file name is missing, assign one based on the app name and upload ID
                    if file_name.is_empty() {
                        file_name = format!("{}_{}", config.app_name, id_string);

                        if mime_type == "application/zip" {
                            file_name.push_str(".zip");
                        }
                    }

                    file_name = encode(&file_name).into_owned();

                    let body = create_body_for(
                        reader, accessor_mutex, db_backend, config);

                    (body, file_name, mime_type)
                },
                // no server-side decryption
                None => {
                    let reader = FileReader::new(
                        &upload_path, start_index, upload.expire_after,
                        upload.is_completed).ok()?;
                    let body = create_body_for(
                        reader, accessor_mutex, db_backend, config);
                    (body, upload.file_name, upload.mime_type)
                }
            };

            Some((body, file_name, mime_type, ciphertext_size))
        }).await
    };

    match response {
        Some((body, file_name, mime_type, ciphertext_size)) => {
            conn
                .with_status(200)
                .with_body(body)
                .with_header("Cache-Control", "no-cache")
                .with_header("Content-Type", mime_type)
                .with_header("Transpo-Ciphertext-Length", format!("{}", ciphertext_size))
                .with_header("Content-Disposition",
                             format!("attachment; filename=\"{}\"", file_name))
                .halt()
        },
        None => error_400(conn, config, translation)
    }
}

fn create_body_for<R>(
    reader: R, accessor_mutex: AccessorMutex,
    db_backend: DbBackend, config: Arc<TranspoConfig>) -> Body
where R: Read + Sync + Send + 'static
{
    let reader = Reader {
        reader,
        accessor_mutex,
        db_backend,
        config
    };

    Body::new_streaming(Unblock::with_capacity(FORM_READ_BUFFER_SIZE, reader), None)
}
