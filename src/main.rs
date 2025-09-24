mod metadata;
mod player;
mod ui;

use anyhow::Result;
use reqwest::Client;
use std::sync::Arc;
use tokio::sync::watch;

#[tokio::main]
async fn main() -> Result<()> {
    let client = Client::builder()
        .user_agent("plaza_term_rs/0.2.0")
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let (tx, rx) = watch::channel(metadata::NowPlaying::default());
    let rx = Arc::new(tokio::sync::Mutex::new(rx));

    let client_for_meta = client.clone();
    let tx_meta = tx.clone();
    tokio::spawn(async move {
        if let Err(e) = metadata::metadata_loop(client_for_meta, tx_meta).await {
            eprintln!("Metadata task error: {:?}", e);
        }
    });

    let stream_url = player::pick_stream(&client).await.unwrap_or_else(|| {
        println!("Using fallback stream URL");
        "http://radio.plaza.one/mp3".to_string()
    });

    println!("🔗 Connecting to: {}", stream_url);

    let (control, sink_info) = player::spawn_ffmpeg_to_rodio(&stream_url).map_err(|e| {
        eprintln!("Failed to start audio player: {}", e);
        eprintln!("Make sure you have audio drivers installed and working");
        e
    })?;

    let ui_result = ui::run_ui(rx, client, control, sink_info).await;

    if let Err(e) = ui_result {
        eprintln!("UI error: {:?}", e);
    } else {
        println!("Thanks for listening to Plaza Radio!");
    }

    Ok(())
}
