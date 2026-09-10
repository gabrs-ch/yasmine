//! Player de música para PC — casca Tauri.
//!
//! A UI é web (`ui/`, React) servida pelo webview do SO; o Rust é o back:
//! índice (`player-core`), áudio (`player-audio`) e a ponte de comandos.
//! Migração de `egui` documentada em `../../design/` e no plano do branch
//! `tauri-ui`.

// Sem console no Windows quando aberto pelo explorador.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|_app| Ok(()))
        .run(tauri::generate_context!())
        .expect("erro ao iniciar o Yasmine");
}
