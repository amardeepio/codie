//! Streaming render benchmark (`TUI-002`, Phase 2 exit criteria).
//!
//! Simulates 5,000 tokens arriving over 60 frames and renders each frame. The
//! budget is one second in total (60 fps), with no frame exceeding 16.7 ms.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use codie_core::{ModelRef, ThinkingLevel};
use codie_tui::app::App;
use codie_tui::keybindings::Bindings;
use codie_tui::render::draw;
use codie_tui::theme::Theme;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

const TOKENS: usize = 5_000;
const FRAMES: usize = 60;
const CHARS_PER_TOKEN: usize = 4;

fn main() {
    let mut app = App::new(
        PathBuf::from("/tmp/codie-bench"),
        ModelRef {
            api: "openai-completions".into(),
            provider: "deepseek".into(),
            model: "deepseek-flash".into(),
        },
        ThinkingLevel::Medium,
        1_000_000,
        Vec::new(),
        Theme::load(Some("dark")).expect("dark theme"),
        None,
        Vec::new(),
        PathBuf::from("/tmp/codie-bench/sessions"),
        Arc::new(std::sync::atomic::AtomicI32::new(0)),
        Bindings::default(),
    );
    app.transcript.start_assistant();

    let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("test terminal");
    let tokens_per_frame = TOKENS / FRAMES;
    let chars_per_frame = tokens_per_frame * CHARS_PER_TOKEN;
    let mut text = String::new();
    let mut frames = Vec::with_capacity(FRAMES);
    let started = Instant::now();

    for frame in 0..FRAMES {
        text.push_str(&format!(
            "frame {frame}: {}\n",
            "streaming output ".repeat(chars_per_frame / 17 + 1)
        ));
        if text.len() > TOKENS * CHARS_PER_TOKEN {
            text.truncate(TOKENS * CHARS_PER_TOKEN);
        }
        app.transcript.set_text(text.clone());
        let frame_started = Instant::now();
        terminal
            .draw(|frame| draw(frame, &mut app))
            .expect("draw frame");
        frames.push(frame_started.elapsed());
    }

    let total = started.elapsed();
    let maximum = frames.iter().copied().max().unwrap_or_default();
    let average = total / u32::try_from(FRAMES).unwrap_or(1);
    let tokens_per_second = TOKENS as f64 / total.as_secs_f64();

    println!("streaming benchmark: {TOKENS} tokens over {FRAMES} frames");
    println!("  total      {:>8.2} ms", total.as_secs_f64() * 1000.0);
    println!(
        "  average    {:>8.2} ms/frame",
        average.as_secs_f64() * 1000.0
    );
    println!(
        "  maximum    {:>8.2} ms/frame",
        maximum.as_secs_f64() * 1000.0
    );
    println!("  throughput {tokens_per_second:>8.0} tokens/s");

    assert!(
        total <= Duration::from_secs(1),
        "60 frames took {total:?}; the 60 fps budget is 1 s"
    );
    assert!(
        maximum <= Duration::from_millis(17),
        "slowest frame took {maximum:?}; the 60 fps budget is 16.7 ms"
    );
}
