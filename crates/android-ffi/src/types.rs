//! Tipos planos que atravessam a FFI. Espelham os do `player-core` /
//! `yasmine-sync`, mas sem os `newtype`s de id (o Kotlin recebe `i64`/`String`
//! cru) e com os hashes já em hex.

use data_encoding::HEXLOWER;

#[derive(uniffi::Enum, Clone, Copy)]
pub enum SortFfi {
    ArtistAlbum,
    Title,
    RecentlyAdded,
}

impl From<SortFfi> for player_core::Sort {
    fn from(s: SortFfi) -> Self {
        match s {
            SortFfi::ArtistAlbum => Self::ArtistAlbum,
            SortFfi::Title => Self::Title,
            SortFfi::RecentlyAdded => Self::RecentlyAdded,
        }
    }
}

#[derive(uniffi::Record)]
pub struct StatsFfi {
    pub tracks: u64,
    pub albums: u64,
    pub artists: u64,
}

impl From<player_core::library::Stats> for StatsFfi {
    fn from(s: player_core::library::Stats) -> Self {
        Self {
            tracks: s.tracks,
            albums: s.albums,
            artists: s.artists,
        }
    }
}

#[derive(uniffi::Record)]
pub struct TrackRowFfi {
    pub id: i64,
    pub title: String,
    pub artist: Option<String>,
    pub artist_id: Option<i64>,
    pub album: Option<String>,
    pub track_no: Option<u32>,
    pub duration_ms: Option<u64>,
    /// Hash da capa em hex, para montar `<cache>/art/<hex[0..2]>/<hex>_96.jpg`.
    pub art_hash: Option<String>,
}

impl From<player_core::TrackRow> for TrackRowFfi {
    fn from(r: player_core::TrackRow) -> Self {
        Self {
            id: r.id.0,
            title: r.title,
            artist: r.artist,
            artist_id: r.artist_id.map(|a| a.0),
            album: r.album,
            track_no: r.track_no,
            duration_ms: r.duration_ms,
            art_hash: r.art_hash.map(|h| HEXLOWER.encode(&h)),
        }
    }
}

#[derive(uniffi::Record)]
pub struct PlaybackInfoFfi {
    pub path: String,
    pub gain_db: Option<f32>,
    pub peak: Option<f32>,
}

impl From<player_core::PlaybackInfo> for PlaybackInfoFfi {
    fn from(p: player_core::PlaybackInfo) -> Self {
        Self {
            path: p.path.to_string_lossy().into_owned(),
            gain_db: p.gain_db,
            peak: p.peak,
        }
    }
}

#[derive(uniffi::Record)]
pub struct PlaylistFfi {
    pub id: String,
    pub name: String,
    pub items: u32,
    pub updated_at: i64,
}

impl From<player_core::playlist::Playlist> for PlaylistFfi {
    fn from(p: player_core::playlist::Playlist) -> Self {
        Self {
            id: p.id.to_string(),
            name: p.name,
            items: p.items as u32,
            updated_at: p.updated_at,
        }
    }
}

// --- sync ---

#[derive(uniffi::Enum, Clone, Copy)]
pub enum SyncPhase {
    Connecting,
    MergingUserData,
    FetchingList,
    Downloading,
    Indexing,
    Done,
}

impl From<yasmine_sync::Phase> for SyncPhase {
    fn from(p: yasmine_sync::Phase) -> Self {
        match p {
            yasmine_sync::Phase::Connecting => Self::Connecting,
            yasmine_sync::Phase::MergingUserData => Self::MergingUserData,
            yasmine_sync::Phase::FetchingList => Self::FetchingList,
            yasmine_sync::Phase::Downloading => Self::Downloading,
            yasmine_sync::Phase::Indexing => Self::Indexing,
            yasmine_sync::Phase::Done => Self::Done,
        }
    }
}

#[derive(uniffi::Record)]
pub struct SyncProgressFfi {
    pub phase: SyncPhase,
    pub tracks_done: u64,
    pub tracks_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub current: Option<String>,
}

impl From<yasmine_sync::Progress> for SyncProgressFfi {
    fn from(p: yasmine_sync::Progress) -> Self {
        Self {
            phase: p.phase.into(),
            tracks_done: p.tracks_done,
            tracks_total: p.tracks_total,
            bytes_done: p.bytes_done,
            bytes_total: p.bytes_total,
            current: p.current,
        }
    }
}

#[derive(uniffi::Record)]
pub struct PullReportFfi {
    pub peer: String,
    pub tracks_added: u64,
    pub bytes: u64,
    pub playlists_merged: u32,
    pub hash_mismatch: u64,
}

impl From<yasmine_sync::PullReport> for PullReportFfi {
    fn from(r: yasmine_sync::PullReport) -> Self {
        Self {
            peer: HEXLOWER.encode(r.peer.as_bytes()),
            tracks_added: r.tracks_added,
            bytes: r.bytes,
            playlists_merged: r.playlists_merged as u32,
            hash_mismatch: r.hash_mismatch,
        }
    }
}

#[derive(uniffi::Record)]
pub struct PairInfoFfi {
    /// Chave pública do host, em hex — é o `DeviceId`.
    pub device_id: String,
    pub name: String,
    pub host: Option<String>,
    pub port: Option<u16>,
}

impl From<yasmine_sync::PairPayload> for PairInfoFfi {
    fn from(p: yasmine_sync::PairPayload) -> Self {
        Self {
            device_id: HEXLOWER.encode(&p.key),
            name: p.name,
            host: p.host,
            port: p.port,
        }
    }
}

#[derive(uniffi::Record)]
pub struct DiscoveredFfi {
    pub device_id: String,
    pub name: String,
    pub addr: String,
}

impl From<yasmine_sync::Discovered> for DiscoveredFfi {
    fn from(d: yasmine_sync::Discovered) -> Self {
        Self {
            device_id: HEXLOWER.encode(d.id.as_bytes()),
            name: d.name,
            addr: d.addr.to_string(),
        }
    }
}

#[derive(uniffi::Record)]
pub struct PairedDeviceFfi {
    pub device_id: String,
    pub name: String,
    pub paired_at: Option<i64>,
    pub last_sync_at: Option<i64>,
}

/// Recebe os progressos do `pull`. O Kotlin implementa isto e joga num
/// `StateFlow` pra Compose.
#[uniffi::export(callback_interface)]
pub trait SyncListener: Send + Sync {
    fn on_progress(&self, progress: SyncProgressFfi);
}
