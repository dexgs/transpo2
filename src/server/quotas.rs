use dashmap::{DashMap, mapref::entry::Entry};
use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::TranspoConfig;


#[derive(Clone)]
pub struct Quotas {
    max_bytes: usize,
    bytes_per_minute: usize,
    quotas: Arc<DashMap<IpAddr, Arc<Mutex<QuotaData>>>>
}

impl From<&TranspoConfig> for Quotas {
    fn from(config: &TranspoConfig) -> Self {
        let this = Self {
            max_bytes: config.quota_bytes_total,
            bytes_per_minute: config.quota_bytes_per_minute,
            quotas: Arc::new(DashMap::new())
        };
        spawn_quotas_thread(this.clone());
        this
    }
}

impl Quotas {
    pub fn get(&self, addr: IpAddr) -> Quota {
        match self.quotas.entry(addr) {
            Entry::Occupied(e) => {
                // Access an existing quota
                let mtx = e.get().clone();
                mtx.lock().unwrap().access();
                Quota { mtx: Some(mtx) }
            },
            Entry::Vacant(e) => {
                // Instantiate a new full quota with 1 accessor
                let inner = QuotaData {
                    num_accessors: 1,
                    bytes_remaining: self.max_bytes,
                    max_bytes: self.max_bytes,
                    bytes_per_minute: self.bytes_per_minute,
                    last_access: Instant::now()
                };
                let mtx = Arc::new(Mutex::new(inner));
                e.insert(mtx.clone());
                Quota { mtx: Some(mtx) }
            }
        }
    }

    fn cleanup(&self) {
        self.quotas.retain(|_, mtx| {
            let mut quota = mtx.lock().unwrap();
            quota.replenish();
            // retain quotas that are being accessed, or are depleted
            quota.num_accessors > 0 || quota.bytes_remaining < self.max_bytes
        })
    }
}

pub struct Quota {
    mtx: Option<Arc<Mutex<QuotaData>>>
}

impl Quota {
    // A "dummy" quota which never runs out
    pub fn unlimited() -> Self {
        Self { mtx: None }
    }

    pub fn lock<'a>(&'a self) -> QuotaGuard<'a> {
        QuotaGuard {
            guard: self.mtx.as_ref().map(|mtx| mtx.lock().unwrap())
        }
    }
}

impl Drop for Quota {
    fn drop(&mut self) {
        if let Some(mut quota) = self.lock().guard {
            quota.release();
        }
    }
}

pub struct QuotaGuard<'a> {
    guard: Option<MutexGuard<'a, QuotaData>>
}

impl<'a> QuotaGuard<'a> {
    // Check if the quota has `num_bytes` remaining
    pub fn check(&mut self, num_bytes: usize) -> bool {
        match &mut self.guard {
            Some(quota) => quota.check(num_bytes),
            None => true
        }
    }

    // Commit to deducting `num_bytes`
    pub fn deduct(&mut self, num_bytes: usize) {
        if let Some(quota) = &mut self.guard {
            quota.deduct(num_bytes);
        }
    }
}

struct QuotaData {
    bytes_remaining: usize,
    max_bytes: usize,
    bytes_per_minute: usize,
    num_accessors: usize,
    last_access: Instant
}

impl QuotaData {
    // Quota accounting

    fn replenish(&mut self) {
        // replenish quota based on elapsed time
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_access);
        self.last_access = now;
        self.bytes_remaining +=
            (elapsed.as_secs() as usize * self.bytes_per_minute) / 60;
        self.bytes_remaining = std::cmp::min(self.bytes_remaining, self.max_bytes);
    }

    fn check(&mut self, num_bytes: usize) -> bool {
        self.replenish();
        self.bytes_remaining >= num_bytes
    }

    fn deduct(&mut self, num_bytes: usize) {
        if self.check(num_bytes) {
            self.bytes_remaining -= num_bytes;
        }
    }


    // Reference counting

    fn access(&mut self) {
        self.num_accessors += 1;
    }

    fn release(&mut self) {
        assert!(self.num_accessors >= 1);
        self.num_accessors -= 1;
    }
}

fn spawn_quotas_thread(quotas: Quotas) {
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(60));
            quotas.cleanup();
        }
    });
}
