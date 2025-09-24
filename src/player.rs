use anyhow::{anyhow, Result};
use rodio::{OutputStream, Sink, Source};
use std::collections::VecDeque;
use std::io::{BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

const STREAM_CANDIDATES: &[&str] = &[
    "http://radio.plaza.one/mp3",
    "http://radio.plaza.one/ogg",
    "http://radio.plaza.one/opus",
];

pub struct SinkInfo {
    pub _channels: u16,
    pub _sample_rate: u32,
}

pub struct PlayerControl {
    pub child: Arc<Mutex<Option<Child>>>,
    pub sink: Arc<Mutex<Sink>>,
    _stream: Arc<OutputStream>, // must keep alive or audio stops
}

impl PlayerControl {
    pub fn stop(&self) {
        if let Ok(s) = self.sink.lock() {
            s.stop();
        }

        if let Ok(mut guard) = self.child.lock() {
            if let Some(mut c) = guard.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }

    pub fn pause(&self) {
        if let Ok(s) = self.sink.lock() {
            s.pause();
        }
    }

    pub fn play(&self) {
        if let Ok(s) = self.sink.lock() {
            s.play();
        }
    }

    pub fn set_volume(&self, vol: f32) {
        if let Ok(s) = self.sink.lock() {
            s.set_volume(vol);
        }
    }

    pub fn is_paused(&self) -> bool {
        if let Ok(s) = self.sink.lock() {
            s.is_paused()
        } else {
            true
        }
    }

    pub fn volume(&self) -> f32 {
        if let Ok(s) = self.sink.lock() {
            s.volume()
        } else {
            0.0
        }
    }
}

pub async fn pick_stream(_client: &reqwest::Client) -> Option<String> {
    STREAM_CANDIDATES.first().map(|s| s.to_string())
}

pub fn spawn_ffmpeg_to_rodio(stream_url: &str) -> Result<(PlayerControl, SinkInfo)> {
    let (stream, stream_handle) = OutputStream::try_default().map_err(|e| {
        anyhow!(
            "Failed to initialize audio output: {}. Check your audio drivers.",
            e
        )
    })?;

    let sink =
        Sink::try_new(&stream_handle).map_err(|e| anyhow!("Failed to create audio sink: {}", e))?;

    sink.set_volume(0.5);

    let sink_arc = Arc::new(Mutex::new(sink));
    let stream_arc = Arc::new(stream);

    let mut child = Command::new("ffmpeg")
        .arg("-reconnect")
        .arg("1")
        .arg("-reconnect_streamed")
        .arg("1")
        .arg("-reconnect_delay_max")
        .arg("5")
        .arg("-i")
        .arg(stream_url)
        .arg("-f")
        .arg("s16le")
        .arg("-acodec")
        .arg("pcm_s16le")
        .arg("-ar")
        .arg("44100")
        .arg("-ac")
        .arg("2")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| anyhow!("Failed to spawn ffmpeg: {}. Is ffmpeg installed?", e))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Failed to capture ffmpeg stdout"))?;

    let (tx, rx) = mpsc::sync_channel::<Vec<i16>>(10);

    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut buf = [0u8; 8192];

        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let mut samples = Vec::with_capacity(n / 2);
                    let mut i = 0usize;
                    while i + 1 < n {
                        let lo = buf[i] as u16;
                        let hi = buf[i + 1] as u16;
                        let sample = ((hi << 8) | lo) as i16;
                        samples.push(sample);
                        i += 2;
                    }

                    if tx.send(samples).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let source = FfmpegSource::new(rx, 2, 44100);
    let sink_for_append = sink_arc.clone();

    thread::spawn(move || {
        if let Ok(sink) = sink_for_append.lock() {
            sink.append(source);
            thread::sleep(std::time::Duration::from_millis(100));
        }
    });

    thread::sleep(std::time::Duration::from_millis(200));

    let control = PlayerControl {
        child: Arc::new(Mutex::new(Some(child))),
        sink: sink_arc,
        _stream: stream_arc,
    };

    Ok((
        control,
        SinkInfo {
            _channels: 2,
            _sample_rate: 44100,
        },
    ))
}

struct FfmpegSource {
    rx: mpsc::Receiver<Vec<i16>>,
    buffer: VecDeque<i16>,
    channels: u16,
    sample_rate: u32,
}

impl FfmpegSource {
    fn new(rx: mpsc::Receiver<Vec<i16>>, channels: u16, sample_rate: u32) -> Self {
        FfmpegSource {
            rx,
            buffer: VecDeque::with_capacity(8192),
            channels,
            sample_rate,
        }
    }
}

impl Iterator for FfmpegSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(s) = self.buffer.pop_front() {
                return Some(s as f32 / 32768.0);
            }

            match self.rx.try_recv() {
                Ok(chunk) => {
                    for v in chunk {
                        self.buffer.push_back(v);
                    }
                    continue;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    if self.buffer.is_empty() {
                        match self.rx.recv_timeout(std::time::Duration::from_millis(100)) {
                            Ok(chunk) => {
                                for v in chunk {
                                    self.buffer.push_back(v);
                                }
                                continue;
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => continue,
                            Err(mpsc::RecvTimeoutError::Disconnected) => return None,
                        }
                    }
                    return Some(0.0);
                }
                Err(mpsc::TryRecvError::Disconnected) => return None,
            }
        }
    }
}

impl Source for FfmpegSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        None // live stream
    }
}
