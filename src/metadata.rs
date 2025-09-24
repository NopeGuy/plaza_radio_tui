use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::interval;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NowPlaying {
    pub artist: Option<String>,
    pub title: Option<String>,
    pub art_url: Option<String>,
}

pub async fn metadata_loop(client: Client, tx: watch::Sender<NowPlaying>) -> Result<()> {
    let primary_url = "https://api.plaza.one/radio/broadcast";
    let fallback_urls = vec![
        "https://api.plaza.one/status",
        "https://api.plaza.one/now_playing",
        "http://radio.plaza.one/status-json.xsl",
    ];

    let mut ticker = interval(Duration::from_secs(5));

    loop {
        ticker.tick().await;

        if let Ok(resp) = client.get(primary_url).send().await {
            if resp.status().is_success() {
                if let Ok(json) = resp.json::<Value>().await {
                    if let Some(np) = parse_plaza_api(&json) {
                        let _ = tx.send(np);
                        continue;
                    }
                }
            }
        }

        for url in &fallback_urls {
            if let Ok(resp) = client.get(*url).send().await {
                if !resp.status().is_success() {
                    continue;
                }
                if let Ok(json) = resp.json::<Value>().await {
                    if let Some(np) = parse_possible_metadata(&json) {
                        let _ = tx.send(np);
                        break;
                    }
                }
            }
        }
    }
}

fn parse_plaza_api(v: &Value) -> Option<NowPlaying> {
    if let Some(np) = v.get("now_playing") {
        return extract_song_info(np);
    }

    if let Some(broadcast) = v.get("broadcast") {
        if let Some(np) = broadcast.get("now_playing") {
            return extract_song_info(np);
        }
    }

    if let Some(current) = v.get("current_song") {
        return extract_song_info(current);
    }

    parse_possible_metadata(v)
}

fn extract_song_info(v: &Value) -> Option<NowPlaying> {
    let artist = v
        .get("artist")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());

    let title = v
        .get("title")
        .or_else(|| v.get("song"))
        .or_else(|| v.get("track"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());

    let art_url = v
        .get("artwork")
        .or_else(|| v.get("artwork_url"))
        .or_else(|| v.get("art"))
        .or_else(|| v.get("cover"))
        .or_else(|| v.get("cover_url"))
        .or_else(|| v.get("image"))
        .or_else(|| v.get("album_art"))
        .and_then(|x| x.as_str())
        .map(|s| {
            if s.starts_with("http://") || s.starts_with("https://") {
                s.to_string()
            } else if s.starts_with("//") {
                format!("https:{}", s)
            } else if s.starts_with("/") {
                format!("https://api.plaza.one{}", s)
            } else {
                format!("https://api.plaza.one/{}", s)
            }
        });

    if artist.is_some() || title.is_some() {
        Some(NowPlaying {
            artist,
            title,
            art_url,
        })
    } else {
        None
    }
}

fn parse_possible_metadata(v: &Value) -> Option<NowPlaying> {
    if v.is_object() {
        let artist = v
            .get("artist")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        let title = v
            .get("title")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        let image = v
            .get("artwork")
            .or_else(|| v.get("image"))
            .or_else(|| v.get("art"))
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());

        if artist.is_some() || title.is_some() || image.is_some() {
            return Some(NowPlaying {
                artist,
                title,
                art_url: image,
            });
        }

        if let Some(cur) = v.get("current").or_else(|| v.get("now_playing")) {
            return extract_song_info(cur);
        }

        // icecast format
        if let Some(icestats) = v.get("icestats") {
            if let Some(source) = icestats.get("source") {
                let s = if source.is_array() {
                    source.get(0).unwrap_or(source)
                } else {
                    source
                };

                let title = s
                    .get("title")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string());

                if let Some(single) = title {
                    if let Some((artist, t)) = single.split_once(" - ") {
                        return Some(NowPlaying {
                            artist: Some(artist.trim().to_string()),
                            title: Some(t.trim().to_string()),
                            art_url: None,
                        });
                    } else {
                        return Some(NowPlaying {
                            artist: None,
                            title: Some(single),
                            art_url: None,
                        });
                    }
                }
            }
        }
    }
    None
}
