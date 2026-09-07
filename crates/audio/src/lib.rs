//! Decode e playback no PC. **Fase 1.**
//!
//! # Decisão: `cpal` + `symphonia` direto, sem `rodio`
//!
//! `rodio` seria menos código, mas paga dois preços que não dá pra tirar
//! depois:
//!
//! 1. **Reamostragem desnecessária.** Ele reamostra sempre que a taxa do
//!    device não bate com a do arquivo, com interpolação linear — gasta CPU e
//!    degrada o áudio. Indo direto no `cpal` dá pra abrir o device *na taxa do
//!    arquivo* quando o hardware aceita, e aí não existe reamostragem no
//!    caminho. É por isso que `sample_rate` é lido do índice antes de abrir o
//!    device, e não descoberto no meio do playback.
//! 2. **Gapless.** Sai quase de graça pré-decodificando a faixa seguinte no
//!    mesmo ring buffer; encaixar isso no modelo de `Sink` do rodio é briga.
//!
//! # Forma
//!
//! Três threads, e a divisão entre elas é o ponto todo:
//!
//! - **Decoder** (worker): `symphonia` decodifica e empurra frames num ring
//!   buffer lock-free (`rtrb`). É onde mora o custo de CPU.
//! - **Callback de áudio** (tempo real, do `cpal`): *só copia bytes* do ring
//!   pro buffer do device. Sem alocar, sem lock, sem I/O, sem `log`. Qualquer
//!   uma dessas coisas aqui vira estalo audível.
//! - **UI**: fala com o engine por comandos, nunca toca no ring.

use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("nenhum dispositivo de áudio disponível")]
    NoDevice,
    #[error("formato não suportado: {0}")]
    UnsupportedFormat(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// O que a UI pode pedir. Fronteira estreita de propósito: mantém o engine
/// substituível e o callback de áudio livre de estado da UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Play,
    Pause,
    Stop,
    Seek(Duration),
    /// Enfileira a próxima faixa para o decoder pré-carregar. É o que faz o
    /// gapless funcionar: quando a atual termina, o áudio da seguinte já está
    /// no ring.
    Preload(std::path::PathBuf),
}

/// Estado observável pela UI. Deliberadamente pequeno e `Copy`: a UI lê isso
/// a ~4 Hz e nunca deve precisar de lock pra desenhar um frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlaybackState {
    pub playing: bool,
    pub position: Duration,
    pub duration: Option<Duration>,
}
