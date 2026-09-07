//! Bindings Kotlin para `player-core` e `player-sync`, via uniffi. **Fase 3.**
//!
//! Divisão decidida: o **ExoPlayer** cuida de decode, playback e serviço em
//! background — é o que dá notificação, tela de bloqueio, Bluetooth e Android
//! Auto de graça e testado. O Rust cuida de indexação, biblioteca e sync.
//!
//! `player-audio` **não** é exposto aqui: no Android quem toca é o ExoPlayer.
//!
//! O Rust é o único escritor do arquivo SQLite; o Kotlin só consulta através
//! desta fronteira.
