//! Tipos que atravessam a ponte IPC. `serde` dos dois lados; espelham os
//! tipos de `player-core` mas com as hashes já em hex (pra `art://`) e os ids
//! como número/string simples.

use serde::{Deserialize, Serialize};

use player_core::library::{ArtistBrief, Stats, TrackRow};
use player_core::playlist::Playlist;

use crate::hexhash;

/// Qual fonte a lista mostra. O front manda `{ kind, id? }`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SourceArg {
    Library,
    Playlist { id: String },
    Artist { id: i64 },
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SortArg {
    #[default]
    ArtistAlbum,
    Title,
    RecentlyAdded,
}

impl From<SortArg> for player_core::Sort {
    fn from(s: SortArg) -> Self {
        match s {
            SortArg::ArtistAlbum => player_core::Sort::ArtistAlbum,
            SortArg::Title => player_core::Sort::Title,
            SortArg::RecentlyAdded => player_core::Sort::RecentlyAdded,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StatsDto {
    pub tracks: u64,
    pub albums: u64,
    pub artists: u64,
}

impl From<Stats> for StatsDto {
    fn from(s: Stats) -> Self {
        Self {
            tracks: s.tracks,
            albums: s.albums,
            artists: s.artists,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackRowDto {
    pub id: i64,
    pub title: String,
    pub artist: Option<String>,
    pub artist_id: Option<i64>,
    pub album: Option<String>,
    pub track_no: Option<u32>,
    pub duration_ms: Option<u64>,
    /// Hex da capa — vira `art://localhost/<art>/96`. `None` = sem capa.
    pub art: Option<String>,
}

impl From<TrackRow> for TrackRowDto {
    fn from(r: TrackRow) -> Self {
        Self {
            id: r.id.0,
            title: r.title,
            artist: r.artist,
            artist_id: r.artist_id.map(|a| a.0),
            album: r.album,
            track_no: r.track_no,
            duration_ms: r.duration_ms,
            art: r.art_hash.as_ref().map(hexhash::encode),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaylistDto {
    pub id: String,
    pub name: String,
    pub items: usize,
    /// Até 4 hashes hex pra montar a miniatura (mosaico) e o hero.
    pub covers: Vec<String>,
    /// A playlist tem pasta vinculada (mostra "· linked folder" no subtítulo).
    pub linked: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtistDto {
    pub id: i64,
    pub name: String,
    pub tracks: u64,
}

impl From<ArtistBrief> for ArtistDto {
    fn from(a: ArtistBrief) -> Self {
        Self {
            id: a.id.0,
            name: a.name,
            tracks: a.tracks,
        }
    }
}

/// Resultado de abrir uma fonte: o que a lista e o hero precisam de imediato.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenResult {
    pub total: usize,
    /// "library" | "playlist" | "artist" — decide se o hero aparece.
    pub kind: &'static str,
    pub title: String,
    pub subtitle: String,
    pub hero_art: Option<String>,
}

/// Helper: `Playlist` + hashes de capa → DTO.
pub fn playlist_dto(pl: &Playlist, covers: &[[u8; 32]], linked: bool) -> PlaylistDto {
    let mut hex: Vec<String> = Vec::with_capacity(4);
    if let Some(h) = pl.image_hash.as_ref() {
        hex.push(hexhash::encode(h));
    }
    for h in covers.iter().take(4 - hex.len().min(4)) {
        hex.push(hexhash::encode(h));
    }
    PlaylistDto {
        id: pl.id.to_string(),
        name: pl.name.clone(),
        items: pl.items,
        covers: hex,
        linked,
    }
}
