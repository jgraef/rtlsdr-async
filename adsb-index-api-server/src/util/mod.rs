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

use bytes::Buf;

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

pub trait BufReadBytesExt {
    fn get_bytes<const N: usize>(&mut self) -> [u8; N];
}

impl<B: Buf> BufReadBytesExt for B {
    fn get_bytes<const N: usize>(&mut self) -> [u8; N] {
        let mut data: [u8; N] = [0; N];
        self.copy_to_slice(&mut data[..]);
        data
    }
}
