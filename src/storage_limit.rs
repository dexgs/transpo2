use std::fs::metadata;
use std::io::Result;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use crate::config::TranspoConfig;


#[derive(Clone)]
pub struct StorageLimit {
    mtx: Option<Arc<Mutex<StorageLimitData>>>
}

impl From<&TranspoConfig> for StorageLimit {
    fn from(config: &TranspoConfig) -> Self {
        if config.max_storage_size_bytes == 0 {
            return Self::unlimited();
        }

        let inner = StorageLimitData {
            max_storage_size: config.max_storage_size_bytes,
            storage_size: get_storage_size(&config.storage_dir).expect("Getting total storage size"),
            write_counter: 0
        };

        let mtx = Arc::new(Mutex::new(inner));
        spawn_storage_limit_thread(mtx.clone(), config.storage_dir.clone());

        Self {
            mtx: Some(mtx)
        }
    }
}

impl StorageLimit {
    // A dummy limit that never runs out
    pub fn unlimited() -> Self {
        Self { mtx: None }
    }

    pub fn lock<'a>(&'a self) -> StorageLimitGuard<'a> {
        StorageLimitGuard {
            guard: self.mtx.as_ref().map(|mtx| mtx.lock().unwrap())
        }
    }
}


pub struct StorageLimitGuard<'a> {
    guard: Option<MutexGuard<'a, StorageLimitData>>
}

impl<'a> StorageLimitGuard<'a> {
    pub fn check(&self, num_bytes: usize) -> bool {
        match &self.guard {
            Some(limit) => limit.check(num_bytes),
            None => true
        }
    }

    pub fn add(&mut self, num_bytes: usize) {
        if let Some(limit) = &mut self.guard {
            limit.add(num_bytes);
        }
    }

    pub fn deduct(&mut self, num_bytes: usize) {
        if let Some(limit) = &mut self.guard {
            limit.deduct(num_bytes);
        }
    }
}

struct StorageLimitData {
    max_storage_size: usize,
    storage_size: usize,
    write_counter: usize
}

impl StorageLimitData {
    fn check(&self, num_bytes: usize) -> bool {
        self.storage_size + num_bytes <= self.max_storage_size
    }

    fn add(&mut self, num_bytes: usize) {
        self.storage_size += num_bytes;
        self.write_counter += num_bytes;
    }

    fn deduct(&mut self, num_bytes: usize) {
        self.storage_size -= num_bytes;
    }
}

fn correct_storage_size<P>(mtx: &Mutex<StorageLimitData>, storage_dir: P) -> Result<()>
where P: AsRef<Path>
{
    /* NOTE: Transpo (currently) does not track upload sizes in any way and
     * relies on the file sizes reported by the filesystem. This works under
     * normal usage, because we can just keep a running count of the total
     * storage size whenever uploads are written/deleted.
     *
     * HOWEVER, what happens if an upload gets deleted from the filesystem
     * (not by Transpo itself)? That storage space will still be considered
     * "used" because Transpo didn't delete the upload, but that space will
     * never become free until Transpo restarts. This means it's technically
     * possible to gradually lose storage space (bad!!!).
     *
     * The implementation here is an attempt to fix this problem without doing
     * something as drastic as periodically preventing writes while we sum up
     * the storage size.
     *
     * The idea is to keep track of the total size of the writes (WITHOUT
     * considering deletes) which take place during summing up the storage size,
     * then add those writes to the `fs_storage_size` and use that as the new
     * storage size if it's lower.
     *
     * This way, we will possibly over-estimate (but NEVER under-estimate) the
     * actual storage space being used, and can switch to using that estimate if
     * it is LOWER than the currently tracked storage size.
     *
     * This means we can reclaim any "phantom" storage space caused by erroneous
     * deletes without any excessive locking OR any chance of under-estimating
     * the storage size and allowing Transpo to store more than the storage limit.
     *
     * TODO: this is not an ideal solution, there's definitely a better option
     * out there...
     */

    // Reset write counter to be able to track the total amount of data written
    // while checking the storage size on the filesystem.
    {
        // The scope here is significant, we want to drop the lock before we
        // compute the total file size
        let mut limit = mtx.lock().unwrap();
        limit.write_counter = 0;
    };
    
    thread::sleep(Duration::from_secs(1));

    // Storage size as reported by the file system
    let fs_storage_size = get_storage_size(storage_dir)?;

    let mut limit = mtx.lock().unwrap();

    let corrected_storage_size = fs_storage_size + limit.write_counter;

    if corrected_storage_size < limit.storage_size {
        eprintln!("Shrinking tracked storage size from {} to {}", limit.storage_size, corrected_storage_size);
        limit.storage_size = corrected_storage_size;
    }

    Ok(())
}

fn get_storage_size<P>(storage_dir: P) -> Result<usize>
where P: AsRef<Path>
{
    let storage_dir = storage_dir.as_ref();

    let mut storage_size = 0;

    for entry in storage_dir.read_dir()? {
        let upload = entry?.path().join("upload");

        if upload.exists() && upload.is_file() {
            storage_size += metadata(upload)?.len() as usize;
        }
    }

    Ok(storage_size)
}

fn spawn_storage_limit_thread(mtx: Arc<Mutex<StorageLimitData>>, storage_dir: PathBuf) {
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(60 * 60));
            if let Err(e) = correct_storage_size(mtx.as_ref(), &storage_dir) {
                eprintln!("Error while correcting storage size: {:?}", e);
            }
        }
    });
}
