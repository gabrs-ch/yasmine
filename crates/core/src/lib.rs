//! Modelo de dados, schema e acesso ao índice da biblioteca.
//!
//! Este crate não sabe tocar áudio nem desenhar UI — ele é a única coisa
//! compartilhada entre o app de PC e o Android (via `player-android-ffi`).

pub mod art;
pub mod db;
pub mod model;
pub mod norm;
pub mod scan;

pub use art::{ArtCache, ArtRef, KnownArt};
pub use db::{Db, Error, Result};
pub use model::{Album, AlbumId, Artist, ArtistId, DeviceId, Track, TrackId, TrackKey};
pub use scan::{ScanReport, scan};
