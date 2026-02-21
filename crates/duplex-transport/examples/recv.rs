use duplex_transport::receiver::Receiver;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let receiver = Receiver::new().expect("create receiver");
    let mut session = receiver
        .connect("127.0.0.1:5000".parse().expect("parse host addr"))
        .await
        .expect("connect host");

    println!("开始接收...");
    for _ in 0..100 {
        let frame = session.recv_video().await.expect("recv video");
        println!(
            "recv: {} bytes keyframe={} ts={}us",
            frame.data.len(),
            frame.is_keyframe,
            frame.timestamp_us
        );
    }
}
