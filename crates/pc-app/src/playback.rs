//! O loop que bombeia os eventos do motor de áudio e transmite o estado de
//! reprodução pro front por `playback://state`. Roda numa thread própria com
//! um `AppHandle`; espelha o antigo `poll_audio` do egui, que rodava a cada
//! frame.

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::dto::TrackRowDto;
use crate::queue::Repeat;
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackDto {
    pub playing: bool,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    pub now: Option<TrackRowDto>,
    /// Posição na fila, contando de 1 (o "4" de "4 / 42").
    pub queue_pos: Option<usize>,
    pub queue_len: usize,
    pub shuffle: bool,
    pub repeat: &'static str,
    pub volume: f32,
}

fn repeat_str(r: Repeat) -> &'static str {
    match r {
        Repeat::Off => "off",
        Repeat::All => "all",
        Repeat::One => "one",
    }
}

/// Fotografa o estado atual. `TrackRow` → DTO só quando a faixa muda seria
/// mais barato, mas é uma struct pequena e isto roda 4×/s.
pub fn snapshot(st: &AppState) -> PlaybackDto {
    let s = st.engine.state();
    PlaybackDto {
        playing: s.playing,
        position_ms: s.position.as_millis() as u64,
        duration_ms: s.duration.map(|d| d.as_millis() as u64),
        now: st.now.clone().map(TrackRowDto::from),
        queue_pos: st.queue.position(),
        queue_len: st.queue.len(),
        shuffle: st.queue.shuffle(),
        repeat: repeat_str(st.queue.repeat()),
        volume: st.engine.volume(),
    }
}

/// Sobe o loop. Emite quando algo muda e, enquanto toca, a cada tick (pra
/// barra de progresso andar).
pub fn spawn(app: AppHandle) {
    std::thread::Builder::new()
        .name("playback".into())
        .spawn(move || {
            let state = app.state::<Mutex<AppState>>();
            let mut last: Option<PlaybackDto> = None;
            loop {
                let dto = {
                    let mut st = state.lock().expect("estado do app");
                    st.pump_audio();

                    // Mexeram na pasta de música: dispara um rescan (o
                    // watcher já espera a rajada acabar antes de sinalizar).
                    if st
                        .watcher
                        .as_ref()
                        .is_some_and(crate::watcher::Watcher::take_change)
                    {
                        crate::scan::spawn(
                            app.clone(),
                            st.paths.db.clone(),
                            st.paths.cache.clone(),
                            st.watcher
                                .as_ref()
                                .expect("checado acima")
                                .root()
                                .to_path_buf(),
                            std::sync::Arc::clone(&st.loudness_running),
                        );
                    }

                    snapshot(&st)
                };
                if dto.playing || last.as_ref() != Some(&dto) {
                    let _ = app.emit("playback://state", &dto);
                    last = Some(dto);
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        })
        .ok();
}
