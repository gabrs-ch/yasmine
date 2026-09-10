import { getCurrentWindow } from "@tauri-apps/api/window";

/** Fora do webview do Tauri (ex.: `vite dev` aberto no navegador pra comparar
 *  com o mockup), os controles de janela viram no-op em vez de estourar. */
export const inTauri =
  typeof (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ !==
  "undefined";

export const appWindow = {
  minimize: () => (inTauri ? getCurrentWindow().minimize() : Promise.resolve()),
  toggleMaximize: () => (inTauri ? getCurrentWindow().toggleMaximize() : Promise.resolve()),
  close: () => (inTauri ? getCurrentWindow().close() : Promise.resolve()),
};
