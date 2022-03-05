use crate::db::*;
use crate::files::*;
use std::thread;
use std::time::Duration;
use std::path::PathBuf;

const CLEANUP_DELAY_SECS: u64 = 60 * 60;

pub fn spawn_cleanup_thread(storage_path: PathBuf, db_backend: DbBackend, db_url: String) {
    thread::spawn(move || cleanup_thread(storage_path, db_backend, db_url));
}

fn cleanup_thread(storage_path: PathBuf, db_backend: DbBackend, db_url: String) {
    loop {
        thread::sleep(Duration::from_secs(CLEANUP_DELAY_SECS));

        let db_connection = establish_connection(db_backend, &db_url);

        if let Some(expired_upload_ids) = Upload::select_expired(&db_connection) {
            for id in expired_upload_ids {
                delete_upload_dir(&storage_path, id);
                Upload::delete_with_id(id, &db_connection);
            }
        }
    }
}
