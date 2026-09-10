//! Estado do back, atrás de um `Mutex` como estado gerenciado do Tauri.
//! É o "lado não-UI" do antigo `App` do egui: banco, áudio, fila, e o que a
//! lista está mostrando agora.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use tauri::AppHandle;
use uuid::Uuid;

use player_audio::Engine;
use player_core::library::{self, PlaybackInfo, TrackRow};
use player_core::{ArtistId, Db, TrackId};

use crate::dto::{SortArg, SourceArg};
use crate::paths::Paths;
use crate::queue::Queue;
use crate::scan;

pub const META_ROOT: &str = "library_root";
pub const META_VOLUME: &str = "volume";

/// Fonte da lista. Espelha o enum do egui; `Copy` de propósito.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Library,
    Playlist(Uuid),
    Artist(ArtistId),
}

impl Source {
    pub fn from_arg(arg: &SourceArg) -> Result<Self, String> {
        Ok(match arg {
            SourceArg::Library => Source::Library,
            SourceArg::Playlist { id } => {
                Source::Playlist(Uuid::parse_str(id).map_err(|_| "uuid inválido".to_string())?)
            }
            SourceArg::Artist { id } => Source::Artist(ArtistId(*id)),
        })
    }
}

pub struct AppState {
    pub db: Db,
    pub paths: Paths,
    pub root: Option<PathBuf>,

    pub source: Source,
    pub sort: SortArg,
    pub query: String,
    /// Ids na ordem atual — a "view". A lista pede janelas dela por índice.
    pub view: Vec<TrackId>,

    pub engine: Engine,
    pub queue: Queue,
    /// Faixa tocando, por id e por linha de exibição (pro `playback://state`).
    pub now_id: Option<TrackId>,
    pub now: Option<TrackRow>,

    /// Guarda do nivelador de loudness — passada pro `scan::spawn`, que
    /// dispara o preenchimento depois de cada scan sem empilhar tarefas.
    pub loudness_running: Arc<AtomicBool>,
}

impl AppState {
    /// Abre banco, lê a raiz e o volume salvos.
    pub fn load() -> Result<Self, String> {
        let paths = Paths::resolve().map_err(|e| e.to_string())?;
        let db = Db::open(&paths.db).map_err(|e| e.to_string())?;

        let meta = |key: &str| -> Option<String> {
            db.conn()
                .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| {
                    r.get::<_, String>(0)
                })
                .ok()
        };
        let root = meta(META_ROOT).map(PathBuf::from).filter(|p| p.is_dir());
        let engine = Engine::new();
        if let Some(v) = meta(META_VOLUME).and_then(|s| s.parse::<f32>().ok()) {
            engine.set_volume(v);
        }

        Ok(Self {
            db,
            paths,
            root,
            source: Source::Library,
            sort: SortArg::default(),
            query: String::new(),
            view: Vec::new(),
            engine,
            queue: Queue::default(),
            now_id: None,
            now: None,
            loudness_running: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Aponta a biblioteca pra `folder`: grava no `meta`, esquece a raiz
    /// anterior, para o que estiver tocando e dispara o scan.
    pub fn set_root(&mut self, app: &AppHandle, folder: PathBuf) {
        let _ = self.db.conn().execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            (META_ROOT, folder.to_string_lossy()),
        );
        let _ = player_core::keep_only_root(&self.db, &folder);
        self.root = Some(folder.clone());
        self.engine.stop();
        self.queue.clear();
        self.now_id = None;
        self.now = None;
        scan::spawn(
            app.clone(),
            self.paths.db.clone(),
            self.paths.cache.clone(),
            folder,
            Arc::clone(&self.loudness_running),
        );
    }

    // ---- playback ----------------------------------------------------------

    /// Toca a faixa no cursor da fila (ou para, se a fila acabou).
    pub fn start_current(&mut self) -> Result<(), String> {
        let Some(id) = self.queue.current() else {
            self.engine.stop();
            self.now_id = None;
            self.now = None;
            return Ok(());
        };
        let info = library::playback_info(&self.db, id)
            .map_err(|e| e.to_string())?
            .ok_or("arquivo da faixa não encontrado")?;
        self.engine.play(info.path.clone(), track_gain(&info));
        self.adopt_current(id);
        Ok(())
    }

    /// Faz `id` ser a faixa tocando e engata a próxima (gapless).
    fn adopt_current(&mut self, id: TrackId) {
        self.now_id = Some(id);
        self.now = library::rows(&self.db, &[id])
            .ok()
            .and_then(|mut rows| rows.pop());
        self.queue_next();
    }

    /// Diz ao motor qual é a próxima faixa, pra ele abrir antes da atual acabar.
    pub fn queue_next(&mut self) {
        let next = self.queue.peek_next().and_then(|id| {
            let info = library::playback_info(&self.db, id).ok().flatten()?;
            Some((info.path.clone(), track_gain(&info)))
        });
        self.engine.set_next(next);
    }

    pub fn play_at(&mut self, index: usize) -> Result<(), String> {
        self.queue.replace(self.view.clone(), index);
        self.start_current()
    }

    pub fn next_track(&mut self) -> Result<(), String> {
        if self.queue.advance().is_some() {
            self.start_current()?;
        }
        Ok(())
    }

    pub fn prev_track(&mut self) -> Result<(), String> {
        if self.queue.previous().is_some() {
            self.start_current()?;
        }
        Ok(())
    }

    pub fn toggle_play(&mut self) -> Result<(), String> {
        if self.engine.state().playing {
            self.engine.pause();
        } else if self.now_id.is_some() && !self.queue.is_empty() {
            self.engine.resume();
        } else {
            self.play_at(0)?;
        }
        Ok(())
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.engine.set_volume(volume.clamp(0.0, 1.0));
        let _ = self.db.conn().execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            (META_VOLUME, self.engine.volume().to_string()),
        );
    }

    /// Drena os eventos do motor — a fila acompanha a emenda gapless que já
    /// aconteceu lá dentro. Chamado pelo loop de `playback.rs`.
    pub fn pump_audio(&mut self) {
        while let Some(event) = self.engine.poll_event() {
            match event {
                player_audio::Event::Advanced { .. } => {
                    self.queue.advance();
                    if let Some(id) = self.queue.current() {
                        self.adopt_current(id);
                    }
                }
                player_audio::Event::Finished => {
                    self.now_id = None;
                    self.now = None;
                }
                player_audio::Event::Started { .. } | player_audio::Event::Error(_) => {}
            }
        }
    }
}

fn track_gain(info: &PlaybackInfo) -> f32 {
    match (info.gain_db, info.peak) {
        (Some(gain_db), Some(peak)) => {
            player_audio::linear_gain(player_audio::Loudness { gain_db, peak })
        }
        _ => 1.0,
    }
}
