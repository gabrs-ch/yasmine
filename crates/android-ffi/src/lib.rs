//! Ponte Kotlin ↔ Rust, via uniffi (modo proc-macro, sem UDL).
//!
//! Divisão decidida no doc de handoff do Yasmine: o **ExoPlayer** cuida de
//! decode, playback e serviço em background (notificação, tela de bloqueio,
//! Bluetooth, Android Auto saem prontos). O Rust cuida de indexação,
//! biblioteca, playlists e **sync**. `player-audio` não aparece aqui.
//!
//! O Rust é o **único escritor** do arquivo SQLite; o Kotlin só consulta
//! através desta fronteira. Toda chamada longa (`scan`, `pull`) é bloqueante —
//! o Kotlin roda em `Dispatchers.IO`.

// A FFI uniffi só aceita parâmetros por valor (`String`, `Vec<_>`,
// `Box<dyn _>`), nunca referência — a sugestão do clippy não se aplica aqui.
#![allow(clippy::needless_pass_by_value)]

mod sync_obj;
mod types;

use std::path::PathBuf;
use std::sync::Mutex;

use player_core::{ArtCache, Db};

pub use types::*;

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum FfiError {
    #[error("{0}")]
    Message(String),
}

impl FfiError {
    fn msg(e: impl std::fmt::Display) -> Self {
        Self::Message(e.to_string())
    }
}

macro_rules! from_err {
    ($($t:ty),*) => {$(
        impl From<$t> for FfiError {
            fn from(e: $t) -> Self { Self::Message(e.to_string()) }
        }
    )*};
}
from_err!(
    player_core::Error,
    yasmine_sync::Error,
    std::io::Error,
    rusqlite::Error,
    uuid::Error
);

type Result<T> = std::result::Result<T, FfiError>;

/// O índice da biblioteca. Uma conexão SQLite atrás de um `Mutex` — as
/// consultas da UI são rápidas e o `pull` segura o lock só nos momentos de
/// merge e indexação.
#[derive(uniffi::Object)]
pub struct YasmineLibrary {
    pub(crate) db: Mutex<Db>,
    pub(crate) art: ArtCache,
    pub(crate) cache_dir: PathBuf,
}

#[uniffi::export]
impl YasmineLibrary {
    /// Abre (criando e migrando) o índice em `db_path`. `cache_dir` é onde as
    /// miniaturas de capa vão parar.
    #[uniffi::constructor]
    pub fn open(db_path: String, cache_dir: String) -> Result<std::sync::Arc<Self>> {
        let db = Db::open(std::path::Path::new(&db_path))?;
        let cache_dir = PathBuf::from(cache_dir);
        std::fs::create_dir_all(&cache_dir)?;
        Ok(std::sync::Arc::new(Self {
            db: Mutex::new(db),
            art: ArtCache::new(cache_dir.clone()),
            cache_dir,
        }))
    }

    /// Varre `root` e atualiza o índice. Devolve quantas faixas entraram ou
    /// mudaram.
    pub fn scan(&self, root: String) -> Result<u32> {
        let mut db = self.lock()?;
        let report = player_core::scan(&mut db, std::path::Path::new(&root), &self.art)?;
        Ok((report.added + report.updated) as u32)
    }

    pub fn stats(&self) -> Result<StatsFfi> {
        let db = self.lock()?;
        Ok(player_core::library::stats(&db)?.into())
    }

    pub fn view(&self, sort: SortFfi) -> Result<Vec<i64>> {
        let db = self.lock()?;
        Ok(id_vec(player_core::library::view(&db, sort.into())?))
    }

    pub fn search(&self, query: String, sort: SortFfi) -> Result<Vec<i64>> {
        let db = self.lock()?;
        Ok(id_vec(player_core::library::search(
            &db,
            &query,
            sort.into(),
        )?))
    }

    pub fn by_artist(&self, artist_id: i64, sort: SortFfi) -> Result<Vec<i64>> {
        let db = self.lock()?;
        Ok(id_vec(player_core::library::by_artist(
            &db,
            player_core::ArtistId(artist_id),
            sort.into(),
        )?))
    }

    /// Linhas de exibição para uma janela de ids, na ordem pedida.
    pub fn rows(&self, ids: Vec<i64>) -> Result<Vec<TrackRowFfi>> {
        let db = self.lock()?;
        let ids: Vec<player_core::TrackId> = ids.into_iter().map(player_core::TrackId).collect();
        Ok(player_core::library::rows(&db, &ids)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Caminho do arquivo e ganho do nivelador — o que o ExoPlayer precisa.
    pub fn playback_info(&self, id: i64) -> Result<Option<PlaybackInfoFfi>> {
        let db = self.lock()?;
        Ok(player_core::library::playback_info(&db, player_core::TrackId(id))?.map(Into::into))
    }

    pub fn playlists(&self) -> Result<Vec<PlaylistFfi>> {
        let db = self.lock()?;
        Ok(player_core::playlist::all(&db)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub fn playlist_tracks(&self, playlist_id: String) -> Result<Vec<i64>> {
        let db = self.lock()?;
        let uuid = uuid::Uuid::parse_str(&playlist_id)?;
        Ok(id_vec(player_core::playlist::tracks(&db, uuid)?))
    }

    pub fn create_playlist(&self, name: String) -> Result<String> {
        let db = self.lock()?;
        Ok(player_core::playlist::create(&db, &name)?.to_string())
    }

    pub fn rename_playlist(&self, id: String, name: String) -> Result<()> {
        let db = self.lock()?;
        player_core::playlist::rename(&db, uuid::Uuid::parse_str(&id)?, &name)?;
        Ok(())
    }

    pub fn delete_playlist(&self, id: String) -> Result<()> {
        let db = self.lock()?;
        player_core::playlist::delete(&db, uuid::Uuid::parse_str(&id)?)?;
        Ok(())
    }

    /// Acrescenta faixas ao fim de uma playlist. Devolve quantas entraram.
    pub fn playlist_append(&self, id: String, track_ids: Vec<i64>) -> Result<u32> {
        let mut db = self.lock()?;
        let uuid = uuid::Uuid::parse_str(&id)?;
        let ids: Vec<player_core::TrackId> =
            track_ids.into_iter().map(player_core::TrackId).collect();
        Ok(player_core::playlist::append(&mut db, uuid, &ids)? as u32)
    }
}

impl YasmineLibrary {
    pub(crate) fn lock(&self) -> Result<std::sync::MutexGuard<'_, Db>> {
        self.db
            .lock()
            .map_err(|_| FfiError::Message("índice travado por um pânico anterior".into()))
    }
}

fn id_vec(ids: Vec<player_core::TrackId>) -> Vec<i64> {
    ids.into_iter().map(|t| t.0).collect()
}
