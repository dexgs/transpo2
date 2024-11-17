use crate::db::*;
use crate::b64::*;
use std::thread;
use std::time::{Duration, SystemTime};
use std::path::{PathBuf, Path};

const CLEANUP_DELAY_SECS: u64 = 60 * 60;

pub fn spawn_cleanup_thread(
    read_timeout_ms: usize, storage_path: PathBuf,
    db_backend: DbBackend, db_url: String)
{
    thread::spawn(move || cleanup_thread(read_timeout_ms, storage_path, db_backend, db_url));
}

pub fn delete_upload<P>(id: i64, storage_path: P, db_connection: &DbConnection)
where P: AsRef<Path> {
    let id_string = String::from_utf8(i64_to_b64_bytes(id)).unwrap();
    let upload_path = storage_path.as_ref().join(id_string);
    if upload_path.exists() {
        // Note: ID generation avoids collisions by checking the
        // filesystem, so we remove the upload directory last.
        Upload::delete_with_id(id, &db_connection);
        if let Err(e) = std::fs::remove_dir_all(&upload_path) {
            eprintln!("Error deleting {:?}: {}", upload_path, e);
        }
    }
}

fn cleanup_thread(
    read_timeout_ms: usize, storage_path: PathBuf,
    db_backend: DbBackend, db_url: String)
{
    loop {
        thread::sleep(Duration::from_secs(CLEANUP_DELAY_SECS));

        let storage_path = storage_path.clone();
        let db_url = db_url.clone();

        thread::spawn(move || cleanup(read_timeout_ms, storage_path, db_backend, db_url));
    }
}

fn cleanup(
    read_timeout_ms: usize, storage_path: PathBuf, db_backend: DbBackend, db_url: String)
{
    let db_connection = establish_connection(db_backend, &db_url);

    if let Some(expired_upload_ids) = Upload::select_expired(&db_connection) {
        for id in expired_upload_ids {
            delete_upload(id, &storage_path, &db_connection);
        }
    }

    // Detect broken uploads by the following criteria:
    // - There is a directory for the upload whose name is a valid ID.
    // - There is no record of an upload with said ID in the database.
    // - The time since the upload was modified *exceeds* the maximum
    //   amount of time Transpo permits between writes, i.e. we can be
    //   reasonably sure that the upload is not currently in progress.
    if let Ok(dir_entries) = std::fs::read_dir(&storage_path) {
        for entry in dir_entries {
            let entry_data = entry.ok()
                .and_then(|e| Some((e.path(), std::fs::metadata(e.path().join("upload")).ok()?)))
                .and_then(|(p, m)| Some((p, m.modified().ok()?)))
                .and_then(|(p, m)| Some((i64_from_b64_bytes(p.file_name()?.to_str()?.as_bytes())?, p, m)));

            if let Some((id, path, modified_time)) = entry_data {
                if path.is_dir() && Upload::select_with_id(id, &db_connection).is_none() {
                    let now = SystemTime::now();
                    if let Ok(age_millis) = now.duration_since(modified_time).map(|d| d.as_millis()) {
                        // Depending on various factors, the modified_time
                        // which gets reported can be slightly behind,
                        // so we give at least 5 seconds of wiggle room.
                        let write_deadline = 5000 + read_timeout_ms;

                        if age_millis as usize > write_deadline
                            && Upload::select_with_id(id, &db_connection).is_none()
                        {
                            delete_upload(id, &storage_path, &db_connection);
                        }
                    }
                }
            }
        }
    }
}
