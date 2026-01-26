use crate::concurrency::*;
use crate::db::*;
use crate::b64::*;
use crate::constants::*;
use crate::config::*;
use crate::files::*;
use crate::http_errors::*;
use crate::translations::*;
use crate::cleanup::delete_upload;
use crate::storage_limit::*;

use std::io::Result;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::pin::{pin, Pin};

use blocking::*;
use trillium::{Conn, Body};
use tokio::io::{AsyncRead, ReadBuf};
use tokio::fs::metadata;
use trillium_tokio::async_compat::Compat;

use urlencoding::{decode, encode};

use argon2::{Argon2, PasswordHash, PasswordVerifier};


struct AsyncReader<R>
where R: AsyncRead {
    reader: R,
    accessor_mutex: Option<AccessorMutex>,
    storage_limit: StorageLimit,
    db_pool: Arc<DbConnectionPool>,
    config: Arc<TranspoConfig>
}

impl<R> AsyncReader<R>
where R: AsyncRead
{
    fn cleanup(&mut self) {
        let config = self.config.clone();
        let storage_limit = self.storage_limit.clone();
        let accessor_mutex = self.accessor_mutex.take().unwrap();
        let db_pool = self.db_pool.clone();
        tokio::spawn(unblock(move || {
            let accessor = accessor_mutex.lock();

            // If we're the last accessor, then it's our responsibility to
            // clean up the upload if it is now invalid!
            if accessor.is_only_accessor() {
                let mut c = db_pool.get().expect("Establishing database connection");

                let should_delete = match Upload::select_with_id(accessor.id, &mut c) {
                    Some(upload) => upload.is_expired(),
                    None => true
                };

                if should_delete {
                    delete_upload(accessor.id, &config.storage_dir, &storage_limit, &mut c);
                }
            }
        }));
    }
}

impl<R> Drop for AsyncReader<R> 
where R: AsyncRead
{
    fn drop(&mut self) {
        self.cleanup();
    }
}

impl<R> AsyncRead for AsyncReader<R>
where R: AsyncRead + Unpin
{
    fn poll_read(
        mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>)
        -> Poll<Result<()>>
    {
        pin!(&mut self.as_mut().reader).poll_read(cx, buf)
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
    id: i64, config: &TranspoConfig, storage_limit: &StorageLimit, accessors: &Accessors,
    db_connection: &mut DbConnection) -> Option<Upload>
{
    let accessor_mutex = accessors.access(id);
    let accessor = accessor_mutex.lock();

    let row = Upload::select_with_id(id, db_connection)?;

    // If the row is expired and we are the only accessor, clean it up!
    let upload = if row.is_expired() {
        if accessor.is_only_accessor() {
            delete_upload(accessor.id, &config.storage_dir, storage_limit, db_connection);
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


// TODO: decide whether or not to remove this
/*
pub async fn info(
    conn: Conn, id_string: String, config: Arc<TranspoConfig>,
    storage_limit: StorageLimit, accessors: Accessors,
    translation: Translation, db_pool: Arc<DbConnectionPool>) -> Conn
{
    if id_string.len() != base64_encode_length(ID_LENGTH) {
        return error_404(conn, config, translation);
    }

    let id = i64_from_b64_bytes(id_string.as_bytes()).unwrap();

    let query = parse_query(conn.querystring());
    let password = query.password;

    let config_ = config.clone();
    let info = pool_unblock!(db_pool, c, {
        let upload = get_upload(id, &config_, &storage_limit, &accessors, &mut c)?;
        let upload_path = config_.storage_dir.join(&id_string).join("upload");
        let ciphertext_size = if upload.is_completed {
            std::fs::metadata(&upload_path).ok()?.len()
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
                .with_response_header("Content-Type", "application/json")
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
*/


async fn get_response_for(
    id_string: String, query: DownloadQuery, config: Arc<TranspoConfig>,
    storage_limit: StorageLimit, accessors: Accessors,
    db_pool: Arc<DbConnectionPool>)
    -> Option<(Body, String, String, usize)>
{
    let upload_path = config.storage_dir.join(&id_string).join("upload");
    if !upload_path.is_file() {
        // Return early if the file does not exist on disk
        return None;
    }

    let id = i64_from_b64_bytes(id_string.as_bytes()).unwrap();

    let crypto_key = query.crypto_key;
    let password = query.password;
    let start_index = query.start_index;

    let (upload, accessor_mutex) = {
        let config = config.clone();
        let storage_limit = storage_limit.clone();
        let db_pool = db_pool.clone();

        pool_unblock!(db_pool, c, {
            let upload = get_upload(id, &config, &storage_limit, &accessors, &mut c)?;

            // validate password
            if !check_password(&password, &upload) {
                return None;
            }

            let accessor_mutex = accessors.access(id);
            Upload::decrement_remaining_downloads(id, &mut c)?;

            Some((upload, accessor_mutex))
        }).await?
    };

    let config = config.clone();

    let ciphertext_size = metadata(&upload_path).await.ok()?.len().try_into().ok()?;

    let (body, file_name, mime_type) = match crypto_key {
        // server-side decryption
        Some(key) => {
            let (reader, mut file_name, mime_type) =
                EncryptedReader::new(
                    &upload_path, start_index, upload.expire_after,
                    upload.is_completed, &key).await.ok()?;

            // If file name is missing, assign one based on the app name and upload ID
            if file_name.is_empty() {
                file_name = format!("{}_{}", config.app_name, id_string);

                let extension = match mime_type.as_ref() {
                    "application/zip" => ".zip",
                    "text/plain" => ".txt",
                    _ => ""
                };
                file_name.push_str(extension);
            }

            file_name = encode(&file_name).into_owned();

            let body = create_async_body_for(
                reader, accessor_mutex, db_pool, config, storage_limit);

            (body, file_name, mime_type)
        },
        // no server-side decryption
        None => {
            let reader = Reader::new(
                &upload_path, start_index, upload.expire_after,
                upload.is_completed).await.ok()?;
            let body = create_async_body_for(
                reader, accessor_mutex, db_pool, config, storage_limit);
            (body, String::from(""), String::from(""))
        }
    };

    Some((body, file_name, mime_type, ciphertext_size))
}

pub async fn handle(
    conn: Conn, id_string: String, config: Arc<TranspoConfig>,
    storage_limit: StorageLimit, accessors: Accessors,
    translation: Translation, db_pool: Arc<DbConnectionPool>) -> Conn
{
    if id_string.len() != base64_encode_length(ID_LENGTH) {
        return error_404(conn, config, translation);
    }

    let query = parse_query(conn.querystring());
    let response = get_response_for(
        id_string, query, config.clone(), storage_limit, accessors, db_pool).await;
    match response {
        Some((body, file_name, mime_type, ciphertext_size)) => {
            conn
                .with_status(200)
                .with_body(body)
                .with_response_header("Cache-Control", "no-cache")
                .with_response_header("Content-Type", mime_type)
                .with_response_header("Transpo-Ciphertext-Length", format!("{}", ciphertext_size))
                .with_response_header("Content-Disposition",
                             format!("attachment; filename=\"{}\"", file_name))
                .halt()
        },
        None => error_400(conn, config, translation)
    }
}

fn create_async_body_for<R>(
    reader: R, accessor_mutex: AccessorMutex,
    db_pool: Arc<DbConnectionPool>, config: Arc<TranspoConfig>,
    storage_limit: StorageLimit) -> Body
where R: AsyncRead + Unpin + Sync + Send + 'static
{
    let reader = AsyncReader {
        reader,
        accessor_mutex: Some(accessor_mutex),
        storage_limit,
        db_pool,
        config
    };

    Body::new_streaming(Compat::new(reader), None)
}
