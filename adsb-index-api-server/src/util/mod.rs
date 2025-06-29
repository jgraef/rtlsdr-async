pub mod json;
pub mod sparse_list;

use std::{
    num::NonZeroUsize,
    sync::{
        OnceLock,
        atomic::{
            AtomicUsize,
            Ordering,
        },
    },
};

pub fn http_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

    CLIENT
        .get_or_init(|| {
            reqwest::ClientBuilder::new()
                .user_agent("jgraef/adsb-index")
                .build()
                .expect("failed to create http client")
        })
        .clone()
}

#[derive(Debug)]
pub struct AtomicIdGenerator {
    next: AtomicUsize,
}

impl Default for AtomicIdGenerator {
    fn default() -> Self {
        Self {
            next: AtomicUsize::new(1),
        }
    }
}

impl AtomicIdGenerator {
    pub fn next(&self) -> NonZeroUsize {
        NonZeroUsize::new(self.next.fetch_add(1, Ordering::Relaxed)).unwrap()
    }
}
