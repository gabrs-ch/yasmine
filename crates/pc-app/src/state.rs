//! Estado do back, atrás de um `Mutex` como estado gerenciado do Tauri.
//! É o "lado não-UI" do antigo `App` do egui: banco, cache de capa, e o que
//! a lista está mostrando agora.

use std::path::PathBuf;

use tauri::AppHandle;
use uuid::Uuid;

use player_core::{ArtistId, Db, TrackId};

use crate::dto::{SortArg, SourceArg};
use crate::paths::Paths;
use crate::scan;

pub const META_ROOT: &str = "library_root";

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
}

impl AppState {
    /// Abre banco + cache, lê a raiz salva no `meta`.
    pub fn load() -> Result<Self, String> {
        let paths = Paths::resolve().map_err(|e| e.to_string())?;
        let db = Db::open(&paths.db).map_err(|e| e.to_string())?;
        let root: Option<PathBuf> = db
            .conn()
            .query_row("SELECT value FROM meta WHERE key = ?1", [META_ROOT], |r| {
                r.get::<_, String>(0)
            })
            .ok()
            .map(PathBuf::from)
            .filter(|p| p.is_dir());
        Ok(Self {
            db,
            paths,
            root,
            source: Source::Library,
            sort: SortArg::default(),
            query: String::new(),
            view: Vec::new(),
        })
    }

    /// Aponta a biblioteca pra `folder`: grava no `meta`, esquece a raiz
    /// anterior e dispara o scan (que emite `scan://progress` / `scan://done`).
    pub fn set_root(&mut self, app: &AppHandle, folder: PathBuf) {
        let _ = self.db.conn().execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            (META_ROOT, folder.to_string_lossy()),
        );
        let _ = player_core::keep_only_root(&self.db, &folder);
        self.root = Some(folder.clone());
        scan::spawn(
            app.clone(),
            self.paths.db.clone(),
            self.paths.cache.clone(),
            folder,
        );
    }
}
