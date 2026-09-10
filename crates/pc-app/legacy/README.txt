// UI egui anterior — referência durante a migração pro Tauri (branch tauri-ui).
// Não entra no build. Fonte da verdade da lógica não-UI que será portada:
//   app.rs  → commands.rs / state.rs (comandos Tauri)
//   theme.rs → ui/src/styles/ (CSS)
//   art.rs  → art_protocol.rs (custom protocol art://)
//   paths.rs  → volta pra src/ na Fase 2 (dir de dados / cache de capa)
//   queue.rs  → volta pra src/ na Fase 3 (shuffle/repeat no AppState)
//   watcher.rs → volta pra src/ na Fase 4 (notify → evento fs)
