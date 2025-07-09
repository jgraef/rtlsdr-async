use rtlsdr_async::devices;

fn main() {
    for device in devices() {
        println!("{device:?}");
    }
}
