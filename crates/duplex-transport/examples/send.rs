use duplex_transport::sender::Sender;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let sender = Sender::bind("0.0.0.0:5000".parse().expect("parse sender addr"))
        .await
        .expect("bind sender");

    println!("等待 Viewer 连接...");
    let mut session = sender.accept().await.expect("accept connection");
    println!("Viewer 已连接，开始发送...");

    // 模拟发送 100 帧
    for i in 0u64..100 {
        let fake_data = vec![0u8; 1024]; // 假数据
        let is_keyframe = i % 30 == 0;
        let timestamp_us = i * 33_333;

        session
            .send_video(fake_data, is_keyframe, timestamp_us)
            .await
            .expect("send video");

        println!("sent frame #{} keyframe={}", i, is_keyframe);
        tokio::time::sleep(tokio::time::Duration::from_millis(33)).await;
    }
}
