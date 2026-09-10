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

// Uma faixa larga e baixa (não um quadrado). O tamanho é fixado por
// min == max; ver `enterMiniWindow` sobre por que NÃO usamos setResizable.
const MINI = { w: 560, h: 116 };
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
  // NÃO chamar setResizable(false) aqui: no GTK, uma janela não-redimensionável
  // ignora as geometry hints e trava no tamanho "natural" do conteúdo (~200px
  // de altura), que era o piso que não dava pra furar. Com a janela
  // redimensionável e min == max, o tamanho fica fixo e o WM respeita os 116px.
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
