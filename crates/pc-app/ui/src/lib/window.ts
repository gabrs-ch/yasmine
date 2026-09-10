import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";

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

const MINI = { w: 380, h: 116 };
const NORMAL_MIN = { w: 640, h: 480 };

/** Encolhe a janela pro modo compacto; devolve o tamanho normal pra restaurar. */
export async function enterMiniWindow(): Promise<[number, number]> {
  if (!inTauri) return [1080, 720];
  const win = getCurrentWindow();

  // Guarda o tamanho normal ANTES de sair de fullscreen/maximizado, senão
  // volta pro tamanho da tela.
  const wasMax = await win.isMaximized();
  const wasFull = await win.isFullscreen();
  const [size, factor] = await Promise.all([win.outerSize(), win.scaleFactor()]);
  const normal: [number, number] =
    wasMax || wasFull ? [1080, 720] : [size.width / factor, size.height / factor];

  // Ordem importa: sair de fullscreen/max primeiro, senão `setSize` é
  // ignorado e a janela fica ocupando a tela inteira.
  if (wasFull) await win.setFullscreen(false);
  if (wasMax) await win.unmaximize();
  await win.setResizable(false);
  await win.setMinSize(new LogicalSize(MINI.w, MINI.h));
  await win.setMaxSize(new LogicalSize(MINI.w, MINI.h));
  await win.setSize(new LogicalSize(MINI.w, MINI.h));
  await win.setAlwaysOnTop(true);
  return normal;
}

export async function exitMiniWindow(normal: [number, number] | null): Promise<void> {
  if (!inTauri) return;
  const win = getCurrentWindow();
  const [w, h] = normal ?? [1080, 720];
  await win.setAlwaysOnTop(false);
  await win.setMaxSize(null);
  await win.setMinSize(new LogicalSize(NORMAL_MIN.w, NORMAL_MIN.h));
  await win.setSize(new LogicalSize(w, h));
  await win.setResizable(true);
}
