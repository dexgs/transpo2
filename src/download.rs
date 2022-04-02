use crate::concurrency::*;
use crate::db::*;
use crate::b64::*;
use crate::constants::*;
use crate::config::*;
use crate::files::*;
use crate::http_errors::*;

use std::io::{Read, Result};
use std::sync::{Arc, Mutex};

use blocking::*;
use trillium::{Conn, Body};

use urlencoding::{decode, encode};

use argon2::{Argon2, PasswordHash, PasswordVerifier};


struct Reader<R>
where R: Read {
    reader: R,
    accessor_mutex: Arc<Mutex<Accessor>>,
    db_backend: DbBackend,
    config: Arc<TranspoConfig>
}

impl<R> Reader<R>
where R: Read
{
    fn cleanup(&mut self) {
        let mut accessor = self.accessor_mutex.lock().unwrap();

        // If we're the last accessor, then it's our responsibility to
        // clean up the upload if it is now invalid!
        if accessor.is_only_accessor() {
            let db_connection = establish_connection(self.db_backend, &self.config.db_url);

            let should_delete = match Upload::select_with_id(accessor.id, &db_connection) {
                Some(upload) => upload.is_expired(),
                None => true
            };

            if should_delete {
                Upload::delete_with_id(accessor.id, &db_connection);
                delete_upload_dir(&self.config.storage_dir, accessor.id);
            }
        }

        accessor.revoke();
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


pub async fn handle(
    conn: Conn, id_string: String, config: Arc<TranspoConfig>,
    accessors: Accessors, db_backend: DbBackend) -> Conn
{
    if id_string.len() != base64_encode_length(ID_LENGTH) {
        return error_404(conn, config);
    }

    let id = i64_from_b64_bytes(id_string.as_bytes()).unwrap();

    let query = conn.querystring();
    
    let mut crypto_key: Option<Vec<u8>> = None;
    let mut password: Option<Vec<u8>> = None;

    // Parse the query string
    for field in query.split('&') {
        if let Some((key, value)) = field.split_once('=') {
            match key {
                "key" => {
                    if value.len() == base64_encode_length(256 / 8) {
                        crypto_key = Some(value.to_owned().into_bytes())
                    }
                },
                "password" => password = decode(value)
                    .ok()
                    .and_then(|s| Some(s.into_owned().into_bytes())),
                _ => return error_400(conn, config)
            }
        }
    }

    let response: Option<(Body, String, String)> = {
        let config = config.clone();
        unblock(move || {
            let db_connection = establish_connection(db_backend, &config.db_url);

            let upload = {
                let accessor_mutex = accessors.access(id, (db_backend, config.db_url.to_owned()));
                let mut accessor = accessor_mutex.lock().unwrap();

                let row = Upload::select_with_id(id, &db_connection)?;

                // If the row is expired and we are the only accessor, clean it up!
                let upload = if row.is_expired() {
                    if accessor.is_only_accessor() {
                        Upload::delete_with_id(accessor.id, &db_connection);
                        delete_upload_dir(&config.storage_dir, accessor.id);
                    }
                    None
                } else {
                    Some(row)
                };

                accessor.revoke();

                upload
            }?;


            // validate password
            let password_hash = upload.password_hash
                .and_then(|h| Some(String::from_utf8(h).unwrap()));
            if let Some(password_hash) = password_hash {
                let parsed_hash = PasswordHash::new(&password_hash).ok()?;
                Argon2::default().verify_password(&password?, &parsed_hash).ok()?;
            }

            Upload::decrement_remaining_downloads(id, &db_connection)?;


            let accessor_mutex = accessors.access(id, (db_backend, config.db_url.to_owned()));
            let upload_path = config.storage_dir.join(&id_string).join("upload");

            let (body, file_name, mime_type) = match crypto_key {
                // server-side decryption
                Some(key) => {
                    let (reader, mut file_name, mime_type) =
                        EncryptedFileReader::new(
                            &upload_path, upload.expire_after, &key, upload.file_name.as_bytes(),
                            upload.mime_type.as_bytes()).ok()?;

                    // If file name is missing, assign one based on the app name and upload ID
                    if file_name.is_empty() {
                        file_name = format!("{}_{}", config.app_name, id_string);

                        if mime_type == "application/zip" {
                            file_name.push_str(".zip");
                        }
                    }

                    file_name = encode(&file_name).into_owned();

                    let body = create_body_for(reader, accessor_mutex, db_backend, config);
                    (body, file_name, mime_type)
                },
                // no server-side decryption
                None => {
                    let reader = FileReader::new(&upload_path, upload.expire_after).ok()?;
                    let body = create_body_for(reader, accessor_mutex, db_backend, config);
                    (body, upload.file_name, upload.mime_type)
                }
            };

            Some((body, file_name, mime_type))
        }).await
    };

    if let Some((body, file_name, mime_type)) = response {
        conn
            .with_status(200)
            .with_body(body)
            .with_header("Cache-Control", "no-cache")
            .with_header("Content-Type", mime_type)
            .with_header("Content-Disposition",
                         format!("attachment; filename=\"{}\"", file_name))
            .halt()
    } else {
        error_400(conn, config)
    }
}

fn create_body_for<R>(
    reader: R, accessor_mutex: Arc<Mutex<Accessor>>,
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
