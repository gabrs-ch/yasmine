//! Tipos do domínio.
//!
//! Os ids inteiros (`TrackId`, `AlbumId`, `ArtistId`) são **locais a um
//! device** — vêm do autoincrement do SQLite. A identidade que atravessa
//! devices é [`TrackKey`], o hash do conteúdo do arquivo. Qualquer coisa que
//! vá pro sync referencia `TrackKey`; qualquer coisa local usa o id, que é
//! oito vezes menor e indexa melhor.

use std::fmt;

macro_rules! row_id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub i64);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

row_id!(
    /// Id local de uma faixa. Cabe em 8 bytes; a lista virtualizada da UI
    /// guarda um `Vec` destes (400 KB pra 50k faixas) em vez das faixas.
    TrackId
);
row_id!(AlbumId);
row_id!(ArtistId);

/// Identidade estável de uma faixa entre devices: BLAKE3 do conteúdo do
/// arquivo. Não inclui as tags, então reeditar metadata não quebra a playlist
/// que aponta pra ela.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TrackKey(pub [u8; 32]);

/// Identidade de um device: a chave pública estática do Noise. Parear e
/// identificar acabam sendo a mesma operação.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeviceId(pub [u8; 32]);

macro_rules! hex_debug {
    ($name:ident) => {
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), self)
            }
        }

        impl fmt::Display for $name {
            /// Só os 8 primeiros bytes: o suficiente pra log, curto o
            /// bastante pra caber numa linha.
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                for b in &self.0[..8] {
                    write!(f, "{b:02x}")?;
                }
                f.write_str("…")
            }
        }

        impl $name {
            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
        }
    };
}

hex_debug!(TrackKey);
hex_debug!(DeviceId);

/// Uma faixa como está no índice. Campos de tag são `Option` porque arquivo
/// sem tag é comum e não pode impedir a faixa de tocar.
#[derive(Debug, Clone)]
pub struct Track {
    pub id: TrackId,
    pub rel_path: String,
    pub file_size: u64,
    pub mtime_ns: i64,
    pub content_hash: Option<TrackKey>,

    pub title: Option<String>,
    pub album_id: Option<AlbumId>,
    pub artist_id: Option<ArtistId>,
    pub disc_no: Option<u32>,
    pub track_no: Option<u32>,
    pub year: Option<i32>,

    pub duration_ms: Option<u64>,
    /// Taxa do arquivo. O engine de áudio abre o device nesta taxa quando o
    /// hardware aceita, e aí não existe reamostragem no caminho.
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub codec: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Album {
    pub id: AlbumId,
    pub title: String,
    pub album_artist_id: Option<ArtistId>,
    pub year: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct Artist {
    pub id: ArtistId,
    pub name: String,
}
