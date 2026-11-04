use std::{thread, time::Duration};

#[used]
static FPS_SIGNATURE: [u8; 27] = *b"%i FPS\0\0%.2fms\0\0Frame : %u\0";

fn main() {
    std::hint::black_box(&FPS_SIGNATURE);
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}
