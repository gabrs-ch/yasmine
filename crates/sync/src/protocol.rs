//! As mensagens que trafegam pelo canal, já cifrado (ver [`crate::channel`]).
//!
//! Serialização: `postcard` — compacto, sem schema no fio, estável entre
//! plataformas. Ordem numa sessão:
//!
//! 1. `Hello` dos dois lados (versão do protocolo).
//! 2. `UserLayer` do host → o celular faz merge antes de qualquer áudio, então
//!    a biblioteca já aparece povoada enquanto os arquivos chegam.
//! 3. `Have` (hashes que o celular tem) → `Tracks` (metadata do que falta).
//! 4. Um `NeedBlob`/`Blob…` por faixa faltante, retomável pelo campo `from`.
//! 5. `Done`.

use serde::{Deserialize, Serialize};

/// Pedaço de áudio por mensagem `Blob`. 128 KiB: poucas idas e voltas sem
/// inflar a memória de quem recebe. O canal Noise fragmenta isso em mensagens
/// de transporte de 64 KiB por baixo — transparente aqui.
pub const CHUNK: usize = 128 * 1024;

/// Teto de um frame lógico desserializado. Guarda contra a outra ponta pedir
/// uma alocação gigante. `UserLayer` de uma biblioteca enorme ainda cabe
/// folgado; áudio nunca vem por aqui, vem em `Blob`.
pub const MAX_FRAME: usize = 64 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
pub enum Msg {
    Hello {
        proto: u16,
        device_name: String,
    },
    /// Snapshot inteiro da camada do usuário do host.
    User(UserLayer),
    /// Hashes de conteúdo que o remetente já tem localmente, ordenados.
    Have {
        hashes: Vec<[u8; 32]>,
    },
    /// Metadata das faixas que o host tem e o par não — a biblioteca aparece
    /// antes dos arquivos terminarem.
    Tracks(Vec<TrackMeta>),
    /// Pede o áudio de um hash a partir do byte `from` (0 = do começo;
    /// diferente de 0 retoma um download interrompido).
    NeedBlob {
        hash: [u8; 32],
        from: u64,
    },
    Blob {
        hash: [u8; 32],
        offset: u64,
        data: Vec<u8>,
        /// `true` no último pedaço deste hash.
        last: bool,
    },
    /// Fim da sessão, tudo que foi pedido já veio.
    Done,
    Error {
        msg: String,
    },
}

/// Metadata de uma faixa, o suficiente para a biblioteca do celular já
/// mostrar título/artista/álbum antes do arquivo chegar. As propriedades de
/// stream e a capa saem do próprio arquivo quando o `scan` roda depois, então
/// não viajam aqui.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackMeta {
    pub hash: [u8; 32],
    /// Extensão do arquivo original (`mp3`, `flac`, …), para nomear o destino.
    pub ext: String,
    pub size: u64,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub disc_no: Option<u32>,
    pub track_no: Option<u32>,
    pub year: Option<i32>,
    pub genre: Option<String>,
}

/// A camada do usuário inteira, como linhas cruas das tabelas. O merge
/// ([`crate::merge`]) decide o que fica.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct UserLayer {
    pub devices: Vec<DeviceRow>,
    pub playlists: Vec<PlaylistRow>,
    pub items: Vec<ItemRow>,
    pub play_counts: Vec<PlayCountRow>,
    pub track_states: Vec<TrackStateRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRow {
    pub id: [u8; 32],
    pub name: String,
    pub is_self: bool,
    pub paired_at: Option<i64>,
    pub last_sync_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistRow {
    pub id: [u8; 16],
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted: bool,
    pub origin: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemRow {
    pub playlist_id: [u8; 16],
    pub position: String,
    pub track_key: [u8; 32],
    pub added_at: i64,
    pub deleted: bool,
    pub deleted_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayCountRow {
    pub track_key: [u8; 32],
    pub device_id: [u8; 32],
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackStateRow {
    pub track_key: [u8; 32],
    pub last_played_at: Option<i64>,
    pub rating: Option<i32>,
    pub rating_updated_at: i64,
    pub resume_pos_ms: Option<i64>,
    pub resume_updated_at: i64,
}
