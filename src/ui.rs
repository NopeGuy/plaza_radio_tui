use crate::metadata::NowPlaying;
use crate::player::PlayerControl;
use anyhow::Result;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use rand::Rng;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Terminal,
};
use reqwest::Client;
use std::io::stdout;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::watch;

pub struct UIState {
    wave_phase: f32,
    last_volume_change: Instant,
    saved_volume: Option<f32>,
}

impl UIState {
    fn new() -> Self {
        Self {
            wave_phase: 0.0,
            last_volume_change: Instant::now(),
            saved_volume: None,
        }
    }
}

pub async fn run_ui(
    rx: Arc<tokio::sync::Mutex<watch::Receiver<NowPlaying>>>,
    _client: Client,
    control: PlayerControl,
    _sink_info: crate::player::SinkInfo,
) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = stdout();
    crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut last_art_url: Option<String> = None;
    let mut art_render: Option<String> = None;
    let mut last_fetch = Instant::now() - Duration::from_secs(3600);
    let mut ui_state = UIState::new();

    loop {
        let np = { rx.lock().await.borrow().clone() };

        let url_opt = np.art_url.clone();
        if url_opt != last_art_url && last_fetch.elapsed() > Duration::from_secs(2) {
            art_render = Some(generate_ascii());

            last_art_url = url_opt.clone();
            last_fetch = Instant::now();
        }

        if art_render.is_none() {
            art_render = Some(generate_ascii());
        }

        terminal.draw(|f| {
            let size = f.size();

            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(40), Constraint::Percentage(60)].as_ref())
                .split(size);

            let left = Paragraph::new(art_render.as_deref().unwrap_or("[loading artwork...]"))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(Color::Magenta)),
                );
            f.render_widget(left, chunks[0]);

            let paused = control.is_paused();
            let current_volume = control.volume();
            let status_icon = if paused { "⏸" } else { "▶" };
            let status_text = if paused { "Paused" } else { "Playing" };

            let wave_visual = generate_waveform(&mut ui_state.wave_phase, !paused, current_volume);
            let volume_bar = generate_pretty_volume_bar(current_volume);
            let volume_recently_changed =
                ui_state.last_volume_change.elapsed() < Duration::from_secs(2);

            let mut lines = vec![];

            lines.push(Line::from(vec![
                Span::raw("Status: "),
                Span::styled(
                    format!("{} {}", status_icon, status_text),
                    if paused {
                        Style::default().fg(Color::Yellow)
                    } else {
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD)
                    },
                ),
            ]));

            lines.push(Line::from(""));

            lines.push(Line::from(vec![
                Span::styled("Title:  ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    np.title.as_deref().unwrap_or("Unknown Title"),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));

            lines.push(Line::from(""));

            lines.push(Line::from(vec![
                if volume_recently_changed {
                    Span::styled("🔊 ", Style::default().fg(Color::Yellow))
                } else {
                    Span::raw("")
                },
                Span::styled("Volume: ", Style::default().fg(Color::Magenta)),
                Span::styled(
                    format!("{:.0}%", current_volume * 100.0),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));

            lines.push(Line::from(volume_bar));
            lines.push(Line::from(""));

            lines.push(Line::from(Span::styled(
                "♫ Waveform ♫",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(wave_visual));
            lines.push(Line::from(""));

            lines.push(Line::from(Span::styled(
                "─── Controls ───",
                Style::default().fg(Color::Gray).add_modifier(Modifier::DIM),
            )));
            lines.push(Line::from(vec![
                Span::styled(
                    "  Space",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" : pause/resume"),
            ]));
            lines.push(Line::from(vec![
                Span::styled(
                    "    +/-",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" : volume up/down"),
            ]));
            lines.push(Line::from(vec![
                Span::styled(
                    "      m",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" : mute/unmute"),
            ]));
            lines.push(Line::from(vec![
                Span::styled(
                    "      q",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" : quit"),
            ]));

            let right = Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(" ☆ Now Playing - Plaza Radio ☆ ")
                    .title_alignment(Alignment::Center)
                    .border_style(Style::default().fg(Color::Cyan)),
            );
            f.render_widget(right, chunks[1]);
        })?;

        if crossterm::event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => {
                        control.stop();
                        break;
                    }
                    KeyCode::Char(' ') => {
                        if control.is_paused() {
                            control.play();
                        } else {
                            control.pause();
                        }
                    }
                    KeyCode::Char('+') | KeyCode::Char('=') => {
                        let new_vol = (control.volume() + 0.1).min(2.0);
                        control.set_volume(new_vol);
                        ui_state.last_volume_change = Instant::now();
                    }
                    KeyCode::Char('-') => {
                        let new_vol = (control.volume() - 0.1).max(0.0);
                        control.set_volume(new_vol);
                        ui_state.last_volume_change = Instant::now();
                    }
                    KeyCode::Char('m') => {
                        let current_volume = control.volume();
                        if current_volume > 0.0 {
                            ui_state.saved_volume = Some(current_volume);
                            control.set_volume(0.0);
                        } else {
                            let restore_volume = ui_state.saved_volume.unwrap_or(0.5);
                            control.set_volume(restore_volume);
                        }
                        ui_state.last_volume_change = Instant::now();
                    }
                    KeyCode::Up => {
                        let new_vol = (control.volume() + 0.05).min(2.0);
                        control.set_volume(new_vol);
                        ui_state.last_volume_change = Instant::now();
                    }
                    KeyCode::Down => {
                        let new_vol = (control.volume() - 0.05).max(0.0);
                        control.set_volume(new_vol);
                        ui_state.last_volume_change = Instant::now();
                    }
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn generate_ascii() -> String {
    let mut art = String::new();

    let lines = [
        "                                                 ",
        "                                                 ",
        "       ╱$$                  ╱$$$$$$              ",
        "      │ $$                 ╱$$__  $$             ",
        "      │ $$       ╱$$   ╱$$│ $$  ╲__╱╱$$$$$$      ",
        "      │ $$      │ $$  │ $$│ $$$$   ╱$$__  $$     ",
        "      │ $$      │ $$  │ $$│ $$_╱  │ $$$$$$$$     ",
        "      │ $$      │ $$  │ $$│ $$    │ $$_____╱     ",
        "      │ $$$$$$$$│  $$$$$$╱│ $$    │  $$$$$$$     ",
        "      │________╱ ╲______╱ │__╱     ╲_______╱     ",
        "                                                 ",
        "                                                 ",
        "                                                 ",
        "                                                 ",
        "                                                 ",
        "                                                 ",
        "                                                 ",
        "                                                 ",
        "                                                 ",
    ];

    let filler_chars = ['¨'];

    let mut processed_lines = Vec::new();
    let mut rng = rand::thread_rng();

    for line in lines.iter() {
        let new_line: String = line
            .chars()
            .map(|c| {
                if c == ' ' {
                    filler_chars[rng.gen_range(0..filler_chars.len())]
                } else {
                    c
                }
            })
            .collect();
        processed_lines.push(new_line);
    }

    let start_color = (255, 140, 0); // orange
    let end_color = (128, 0, 128); // urple
    let n = processed_lines.len() as f32;

    for (i, line) in processed_lines.iter().enumerate() {
        let t = i as f32 / (n - 1.0);
        let r = (start_color.0 as f32 * (1.0 - t) + end_color.0 as f32 * t) as u8;
        let g = (start_color.1 as f32 * (1.0 - t) + end_color.1 as f32 * t) as u8;
        let b = (start_color.2 as f32 * (1.0 - t) + end_color.2 as f32 * t) as u8;
        art.push_str(&format!("\x1b[38;2;{};{};{}m{}\n", r, g, b, line));
    }

    art.push_str("\x1b[0m");
    art
}

fn generate_waveform(phase: &mut f32, is_playing: bool, volume: f32) -> String {
    let bar_count = 40;
    let mut rng = rand::thread_rng();

    if is_playing {
        *phase += 0.2;
    } else {
        *phase *= 0.95;
    }

    let mut bars = String::new();

    for i in 0..bar_count {
        let x = i as f32 / bar_count as f32;

        let wave1 = ((*phase + x * 8.0).sin() * 0.3 + 0.5).abs();
        let wave2 = (((*phase * 1.3) + x * 12.0).sin() * 0.2 + 0.5).abs();
        let wave3 = (((*phase * 0.7) + x * 4.0).cos() * 0.3 + 0.5).abs();

        let noise = rng.gen_range(-0.1..0.1);
        let combined = (wave1 + wave2 + wave3) / 3.0 + noise;
        let level = (combined * volume * 20.0).clamp(0.0, 8.0) as u8;

        let final_level = if volume == 0.0 || !is_playing {
            (level as f32 * 0.2) as u8
        } else {
            level
        };

        let bar_char = match final_level {
            0 => '▁',
            1 => '▂',
            2 => '▃',
            3 => '▄',
            4 => '▅',
            5 => '▆',
            6 => '▇',
            _ => '█',
        };

        bars.push(bar_char);
    }

    bars
}

fn generate_pretty_volume_bar(volume: f32) -> String {
    let vol_percent = (volume * 100.0) as usize;
    let bar_length = 20;
    let filled = (vol_percent * bar_length / 100).min(bar_length);

    let mut bar = String::new();
    bar.push_str("│");

    for i in 0..bar_length {
        if i < filled {
            if volume == 0.0 {
                bar.push('✗');
            } else if i < bar_length * 60 / 100 {
                bar.push('▓');
            } else if i < bar_length * 80 / 100 {
                bar.push('▒');
            } else {
                bar.push('░');
            }
        } else {
            bar.push('·');
        }
    }

    bar.push_str("│");

    if volume == 0.0 {
        bar.push_str(" 🔇");
    } else if vol_percent < 30 {
        bar.push_str(" 🔈");
    } else if vol_percent < 70 {
        bar.push_str(" 🔉");
    } else {
        bar.push_str(" 🔊");
    }

    bar
}
