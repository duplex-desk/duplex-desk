use std::sync::OnceLock;
use std::time::Instant;

static MONO_START: OnceLock<Instant> = OnceLock::new();

pub fn mono_now_us() -> u64 {
    let elapsed = MONO_START.get_or_init(Instant::now).elapsed().as_micros();
    if elapsed > u64::MAX as u128 {
        u64::MAX
    } else {
        elapsed as u64
    }
}
