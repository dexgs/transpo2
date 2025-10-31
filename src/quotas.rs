use dashmap::DashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

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
