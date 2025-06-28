pub mod json;
pub mod sparse_list;

use std::sync::OnceLock;

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
