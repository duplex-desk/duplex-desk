use duplex_scap::{capturer::ScreenCapturer, config::DuplexScapConfig};

fn main() {
    if !ScreenCapturer::check_permissions() {
        println!("need permissions, requesting...");
        ScreenCapturer::request_permissions();
        return;
    }

    let mut capturer = ScreenCapturer::new();
    let rx = capturer.start(DuplexScapConfig::default()).unwrap();

    println!("started capturing, receiving frames...");
    for i in 0..60 {
        let frame = rx.recv().unwrap();
        println!(
            "frame {:02}: {}x{} stride={} ts={}us ({} bytes)",
            i,
            frame.width,
            frame.height,
            frame.stride,
            frame.timestamp_us,
            frame.data.len()
        );
    }

    capturer.stop().unwrap();
    println!("stopped capturing");
}
