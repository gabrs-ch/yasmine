//! App de PC. **Fase 1.**
//!
//! UI decidida: `egui`/`eframe` com backend `glow` (OpenGL), não `wgpu` —
//! criar o contexto GL é mais rápido e traz menos dependência, o que aparece
//! direto no cold start.
//!
//! Duas regras que vêm junto com a escolha, e que são metade do custo de CPU
//! de um player em repouso:
//!
//! 1. Modo reativo: repaint só em evento. O padrão de redesenhar a 60 fps
//!    deixa o processo consumindo CPU parado numa lista de músicas.
//! 2. Tocando, repaint limitado a ~4 Hz — só o suficiente pra barra de
//!    progresso andar. Não 60.

fn main() {
    println!("Fase 1: seleção de pasta, scan, índice e playback.");
}
