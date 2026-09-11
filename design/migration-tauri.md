# Migração da UI do PC: egui → Tauri

Registro de como a migração foi feita, escrito na época (v0.5.0). Descreve
o estado **daquele momento** — o crate de sync ainda se chamava
`player-sync` e era um stub, e o app Android ainda não existia. Pra
arquitetura atual, ver [`docs/arquitetura.md`](../docs/arquitetura.md).

`mockup.html` neste diretório continua sendo a fonte da verdade visual —
abrir no navegador e comparar lado a lado com `cargo tauri dev`.

## Por quê

O `egui` (modo imediato) não alcança a qualidade do mockup: sem
`letter-spacing`, sem transição/animação CSS, gradiente só na mão, layout
difícil de centrar, AA de texto inferior. O mockup é HTML/CSS — no webview
ele *é* o código.

## O que mudou

- `crates/pc-app` deixou de ser `egui`/`eframe` e virou app **Tauri 2**:
  back Rust + front web (`crates/pc-app/ui`, React + TypeScript + Vite).
- `player-core`, `player-audio`, `player-sync` e o app **Android** não mudaram.

## Forma

- **Front** (`ui/`): CSS portado verbatim do mockup; `zustand` pro estado;
  `@tanstack/react-virtual` na lista; `lucide-react` nos ícones; fontes
  locais (sem CDN). Menu de contexto próprio (o nativo do WebKit destoaria).
- **Back** (`src/`): `AppState` atrás de `Mutex` como estado gerenciado do
  Tauri — banco, `Engine`, `Queue`, cache de capa, watcher. Comandos
  `#[tauri::command]` traduzem pra `player-core`. Protocolo `art://` serve as
  miniaturas. Loop de `playback.rs` transmite `playback://state` (250 ms) e
  dispara rescan quando o watcher sinaliza. Scan em thread com eventos
  `scan://…`.
- A lógica não-UI (fila, playback, scan) é porte 1:1 do antigo `App` do egui.

## Empacotamento

- **Linux**: AppImage com WebKitGTK embutido (`linuxdeploy-plugin-gtk`) —
  ~81 MB, um arquivo, sem root. `.deb` também, pra quem usa apt.
- **Windows**: `.zip` portátil (`.exe` cru) + instalador NSIS (`currentUser`).
  WebView2 via `downloadBootstrapper` — não infla o pacote; o Win11 já traz.

## Custo assumido

+RAM (~50 → ~150 MB), cold start mais lento que `eframe` + `glow`, dois
toolchains (cargo + npm), e a dependência de runtime do webview.
