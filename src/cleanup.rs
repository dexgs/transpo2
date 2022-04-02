use crate::db::*;
use crate::files::*;
use std::thread;
use std::time::{Duration, SystemTime};
use std::path::PathBuf;

const CLEANUP_DELAY_SECS: u64 = 60 * 60;

pub fn spawn_cleanup_thread(
    max_upload_age_minutes: usize, storage_path: PathBuf,
    db_backend: DbBackend, db_url: String)
{
    thread::spawn(move || cleanup_thread(max_upload_age_minutes, storage_path, db_backend, db_url));
}

fn cleanup_thread(
    max_upload_age_minutes: usize, storage_path: PathBuf,
    db_backend: DbBackend, db_url: String)
{
    loop {
        thread::sleep(Duration::from_secs(CLEANUP_DELAY_SECS));

        let storage_path = storage_path.clone();
        let db_url = db_url.clone();

        let join_handle = thread::spawn(move || {
            let db_connection = establish_connection(db_backend, &db_url);

            if let Some(expired_upload_ids) = Upload::select_expired(&db_connection) {
                for id in expired_upload_ids {
                    delete_upload_dir(&storage_path, id);
                    Upload::delete_with_id(id, &db_connection);
                }
            }

            // Delete any uploads older than the max upload time. This is to make
            // sure uploaded files that do not occur in the database get cleaned up.
            if let Ok(dir_entries) = std::fs::read_dir(&storage_path) {
                for entry in dir_entries {
                    if let Ok((path, modified_time)) = entry
                        .and_then(|e| Ok((e.path(), std::fs::metadata(e.path())?)))
                        .and_then(|(p, m)| Ok((p, m.modified()?)))
                    {
                        if path.is_dir() {
                            let now = SystemTime::now();
                            if let Ok(age) = now.duration_since(modified_time) {
                                let age_minutes = age.as_secs() as usize / 60;
                                if age_minutes > max_upload_age_minutes {
                                    if let Err(e) = std::fs::remove_dir_all(path) {
                                        eprintln!("{}", e);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
        if join_handle.join().is_err() {
            eprintln!("Joining cleanup thread failed");
        }
    }
}
