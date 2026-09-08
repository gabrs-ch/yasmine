//! Decode e playback no PC.
//!
//! # Decisão: `cpal` + `symphonia` direto, sem `rodio`
//!
//! O `rodio` seria menos código, mas cobra dois preços que não dá pra tirar
//! depois:
//!
//! 1. **Reamostragem desnecessária.** Ele reamostra sempre que a taxa do
//!    dispositivo não bate com a do arquivo, com interpolação linear — gasta
//!    CPU e degrada o áudio. Indo direto no `cpal` dá pra abrir o dispositivo
//!    *na taxa do arquivo* quando o hardware aceita, e aí não existe
//!    reamostragem no caminho.
//! 2. **Gapless.** Sai quase de graça pré-decodificando a próxima faixa no
//!    mesmo ring buffer; encaixar isso no modelo de `Sink` do rodio é briga.
//!
//! # Forma
//!
//! Três threads, e a divisão entre elas é o ponto todo:
//!
//! - **Worker** ([`engine`]): decodifica, converte se precisar e empurra
//!   samples no ring. É onde mora o custo de CPU.
//! - **Callback de áudio** (tempo real, do `cpal`): *só copia bytes* do ring
//!   pro buffer do dispositivo. Sem alocar, sem lock, sem I/O, sem log.
//! - **UI**: manda comando e lê átomos. Nunca toca no ring.

pub mod convert;
pub mod decode;
pub mod engine;
pub mod loudness;
pub mod output;

use std::path::PathBuf;
use std::time::Duration;

pub use decode::{Spec, TrackDecoder};
pub use engine::Engine;
pub use loudness::{Loudness, linear_gain};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("nenhum dispositivo de áudio disponível")]
    NoDevice,

    #[error("o arquivo não tem faixa de áudio")]
    NoAudioTrack,

    #[error("formato não suportado: {0}")]
    UnsupportedFormat(String),

    #[error("dispositivo de áudio: {0}")]
    Device(String),

    #[error("erro de e/s: {0}")]
    Io(#[from] std::io::Error),

    #[error("erro ao decodificar: {0}")]
    Decode(#[from] symphonia::core::errors::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// O que a UI pode pedir. Fronteira estreita de propósito: mantém o worker
/// substituível e o callback de áudio livre de estado da UI.
///
/// `f32` no lugar de `Eq`: `Play`/`SetNext` carregam o ganho do nivelador já
/// resolvido em linear (`loudness::linear_gain`). Quem chama (o app) é quem
/// sabe o `gain_db`/pico de cada faixa no índice — o motor só aplica.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Caminho e ganho linear a aplicar (1.0 = sem ajuste).
    Play(PathBuf, f32),
    /// Qual faixa vem depois, com o ganho dela. É o que permite o gapless: o
    /// worker abre a próxima antes de a atual acabar.
    SetNext(Option<(PathBuf, f32)>),
    Pause,
    Resume,
    Stop,
    Seek(Duration),
}

/// Avisos do worker para a UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Started {
        path: PathBuf,
    },
    /// Uma faixa emendada começou a tocar de fato.
    Advanced {
        path: PathBuf,
    },
    /// A fila acabou e o dispositivo já drenou.
    Finished,
    Error(String),
}

/// Estado observável pela UI. Pequeno e `Copy` de propósito: é lido a cada
/// frame e nunca deve precisar de lock pra desenhar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlaybackState {
    pub playing: bool,
    pub position: Duration,
    pub duration: Option<Duration>,
}
