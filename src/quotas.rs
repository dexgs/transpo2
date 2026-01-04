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
    quotas: Arc<DashMap<IpAddr, usize>>,
}

impl From<&TranspoConfig> for Quotas {
    fn from(config: &TranspoConfig) -> Self {
        Self {
            max_bytes: config.quota_bytes_total,
            bytes_per_minute: config.quota_bytes_per_minute,
            quotas: Arc::new(DashMap::new())
        }
    }
}

impl Quotas {
    // Return whether or not writing the given amount of bytes would exceed
    // the quota for the given address
    pub fn exceeds_quota(&self, addr: &IpAddr, bytes: usize) -> bool {
        let count = match self.quotas.get_mut(addr) {
            Some(mut count) => {
                *count += bytes;
                *count
            },
            None => {
                self.quotas.insert(addr.to_owned(), bytes);
                bytes
            }
        };

        count > self.max_bytes
    }

    fn replenish(&self) {
        self.quotas.retain(|_, count| *count > self.bytes_per_minute);

        for mut count in self.quotas.iter_mut() {
            *count -= self.bytes_per_minute;
        }
    }
}

pub fn spawn_quotas_thread(quotas: Quotas) {
    thread::spawn(move || quotas_thread(quotas));
}

fn quotas_thread(quotas: Quotas) {
    loop {
        thread::sleep(Duration::from_secs(60));
        quotas.replenish();
    }
}

#[derive(Clone)]
pub struct NewQuotas {
    max_bytes: usize,
    bytes_per_minute: usize,
    quotas: Arc<DashMap<IpAddr, Arc<Mutex<QuotaData>>>>
}

impl From<&TranspoConfig> for NewQuotas {
    fn from(config: &TranspoConfig) -> Self {
        Self {
            max_bytes: config.quota_bytes_total,
            bytes_per_minute: config.quota_bytes_per_minute,
            quotas: Arc::new(DashMap::new())
        }
    }
}

impl NewQuotas {
    pub fn get(&mut self, addr: IpAddr) -> Quota {
        match self.quotas.entry(addr) {
            Entry::Occupied(e) => {
                // Access an existing quota
                let mtx = e.get().clone();
                mtx.lock().unwrap().access();
                Quota { mtx: Some(mtx) }
            },
            Entry::Vacant(e) => {
                // Instantiate a new quota with 1 accessor and all bytes remaining
                let inner = QuotaData {
                    num_accessors: 1,
                    bytes_remaining: self.max_bytes,
                    max_bytes: self.max_bytes,
                    bytes_per_minute: self.bytes_per_minute,
                    last_access: Instant::now(),
                    addr,
                    quotas: self.quotas.clone()
                };
                let mtx = Arc::new(Mutex::new(inner));
                e.insert(mtx.clone());
                Quota { mtx: Some(mtx) }
            }
        }
    }
}

pub struct Quota {
    mtx: Option<Arc<Mutex<QuotaData>>>
}

impl Quota {
    pub fn lock<'a>(&'a self) -> QuotaGuard<'a> {
        match &self.mtx {
            Some(mtx) => QuotaGuard { guard: Some(mtx.lock().unwrap()) },
            None => QuotaGuard { guard: None }
        }
    }
}

impl Drop for Quota {
    fn drop(&mut self) {
        let guard = self.lock();
        if let Some(mut quota) = guard.guard {
            quota.release();
            quota.quotas.remove_if(&quota.addr, |_, _| quota.num_accessors == 0);
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
    last_access: Instant,
    addr: IpAddr,
    quotas: Arc<DashMap<IpAddr, Arc<Mutex<QuotaData>>>>
}

impl QuotaData {
    // Quota accounting

    fn check(&mut self, num_bytes: usize) -> bool {
        // replenish quota based on elapsed time
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_access);
        self.last_access = now;
        self.bytes_remaining +=
            (elapsed.as_secs() as usize * self.bytes_per_minute) / 60;
        self.bytes_remaining = std::cmp::min(self.bytes_remaining, self.max_bytes);

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
