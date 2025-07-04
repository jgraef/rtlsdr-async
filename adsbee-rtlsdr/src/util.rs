use bytes::Buf;

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
