//! Toca arquivos de verdade no dispositivo de verdade e relata o que aconteceu.
//!
//! Passando mais de um arquivo, o segundo é emendado no primeiro pelo caminho
//! do gapless — é o teste que interessa.
//!
//! ```text
//! cargo run --release -p player-audio --example play -- a.mp3 b.mp3
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};

use player_audio::{Engine, Event};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let files: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    if files.is_empty() {
        return Err("uso: play <arquivo> [próximo…]".into());
    }

    let engine = Engine::new();
    engine.play(files[0].clone());

    let mut fila = files[1..].iter().cloned();
    engine.set_next(fila.next());

    let started = Instant::now();
    let mut starvations = 0u32;
    let limite = Duration::from_secs(30);

    loop {
        while let Some(event) = engine.poll_event() {
            let t = started.elapsed().as_secs_f64();
            match event {
                Event::Started { path } => println!("[{t:5.2}s] iniciou   {}", nome(&path)),
                Event::Advanced { path } => {
                    println!("[{t:5.2}s] emendou   {}", nome(&path));
                    engine.set_next(fila.next());
                }
                Event::Finished => {
                    let state = engine.state();
                    println!("[{t:5.2}s] terminou  (posição final {:?})", state.position);
                    println!("\nunderruns: {starvations}");
                    return Ok(());
                }
                Event::Error(err) => {
                    eprintln!("[{t:5.2}s] erro: {err}");
                    return Err(err.into());
                }
            }
        }

        if engine.take_starved() {
            starvations += 1;
        }

        let state = engine.state();
        print!(
            "\r  {:6.2}s / {:<8}  {}   ",
            state.position.as_secs_f64(),
            state
                .duration
                .map_or_else(|| "?".to_owned(), |d| format!("{:.2}s", d.as_secs_f64())),
            if state.playing { "tocando" } else { "parado " }
        );
        use std::io::Write as _;
        std::io::stdout().flush().ok();

        if started.elapsed() > limite {
            return Err("nada terminou em 30s".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn nome(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into(),
    )
}
