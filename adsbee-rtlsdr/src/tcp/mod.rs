pub mod client;
pub mod server;

#[derive(Clone, Copy, Debug)]
pub struct DongleInfo {
    pub magic: [u8; 4],
    pub tuner_type: u32,
    pub tuner_gain_type: u32,
}
