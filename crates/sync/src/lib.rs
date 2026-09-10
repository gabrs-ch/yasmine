//! Pareamento por QR, descoberta na LAN e transferência de biblioteca entre
//! devices. **Fase 4 do Yasmine**, implementada.
//!
//! Crate isolado de propósito: é o único que fala com a rede e o único que
//! mexe com cripto, então merece revisão mais cuidadosa que o resto.
//!
//! # As duas camadas, dois algoritmos
//!
//! - **Camada derivada (áudio):** endereçada por conteúdo (BLAKE3). O
//!   protocolo é "tenho / não tenho este hash" — conflito não existe. O
//!   celular grava os arquivos numa pasta local e roda [`player_core::scan`]
//!   nela como se o usuário a tivesse apontado.
//! - **Camada do usuário (playlists, plays, rating):** merge de verdade. LWW
//!   por campo para rating e posição de retomada, `MAX` por `(faixa, device)`
//!   para contagem de plays, túmulo para item e playlist apagados. Ver
//!   [`merge`].
//!
//! # O caminho
//!
//! ```text
//! celular                                   PC (host)
//!   │  lê QR: chave estática + host:porta       │
//!   │─────────── TCP + Noise_IK ───────────────▶│   handshake autentica os dois
//!   │◀────────── UserLayer (snapshot) ──────────│   camada do usuário primeiro (pequena)
//!   │─────────── Have { hashes locais } ───────▶│
//!   │◀────────── Tracks { metadata do que falta }│
//!   │─────────── NeedBlob { hash, from } ──────▶│   um por faixa, retomável
//!   │◀────────── Blob { chunks } ───────────────│
//!   │  verifica BLAKE3, grava em dest/…         │
//!   │  scan(dest) + ensure_hashes              │   playlists encaixam por track_key
//! ```

mod channel;
mod client;
mod discovery;
mod identity;
mod merge;
mod pairing;
mod protocol;
mod server;

pub use client::{Phase, Progress, PullReport, Target, pull};
pub use discovery::{Advertisement, Discovered, Discovery, advertise, resolve_once};
pub use identity::Identity;
pub use merge::apply as apply_user_layer;
pub use pairing::PairPayload;
pub use protocol::UserLayer;
pub use server::{AuthDecision, Server, ServerEvent};

pub use player_core::DeviceId;

/// Versão do protocolo de fio. Sobe quando um `Msg` muda de forma de um jeito
/// que a ponta antiga não entende.
pub const PROTO_VERSION: u16 = 1;

/// Tipo de serviço mDNS. O host publica isto; o celular navega por isto e casa
/// pelo `id` (chave pública) no TXT.
pub const SERVICE_TYPE: &str = "_yasmine-sync._tcp.local.";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("índice: {0}")]
    Core(#[from] player_core::Error),

    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("e/s: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialização: {0}")]
    Wire(#[from] postcard::Error),

    #[error("handshake Noise falhou: {0}")]
    Noise(String),

    #[error("mDNS: {0}")]
    Mdns(String),

    /// O device do outro lado não está na tabela `device` deste. Quem chama
    /// (host) decide se pergunta pro usuário e pareia, ou recusa.
    #[error("device não pareado: {0}")]
    Unpaired(DeviceId),

    /// A chave que respondeu não é a que o QR prometeu — device errado ou
    /// alguém no meio.
    #[error("a chave do host não confere com a do QR")]
    PeerMismatch,

    #[error("a outra ponta falou fora do protocolo: {0}")]
    Protocol(String),

    #[error("o arquivo baixado não bateu com o hash pedido")]
    HashMismatch,

    #[error("QR inválido: {0}")]
    BadPairUrl(&'static str),

    #[error("cancelado")]
    Cancelled,
}

impl From<snow::Error> for Error {
    fn from(e: snow::Error) -> Self {
        Self::Noise(e.to_string())
    }
}

impl From<mdns_sd::Error> for Error {
    fn from(e: mdns_sd::Error) -> Self {
        Self::Mdns(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
