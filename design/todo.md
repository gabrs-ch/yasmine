# Ajustes pendentes da UI

## Feito (no branch, aguardando build/tag)

- **Equalizer para quando pausa.** `<Equalizer paused>` + `.eq.paused i`
  (barras baixas, `animation: none`). Casado com `playback.playing`.
- **Mutar no ícone de volume.** Ícone vira `<button>` → `toggleMute`
  (guarda o volume anterior num módulo). `VolumeX` quando mudo.
- **Bolinha na barra de volume** ao passar o mouse (`knob` no `<Scrubber>`
  + `.vol-line:hover .track-knob`).
- **Título/álbum do now-playing usa mais largura.** `.np` e `.right` viram
  `flex: 1 1 0` (crescem igual → transporte segue centrado); `.bar` fixa em
  `min(560px, 42vw)`. Só corta quando encostaria no transporte.

## Pendente

- **Modo compacto ainda deixa a janela "grossa"** no KWin do usuário. O
  v0.5.2 já sai de fullscreen/maximizado antes e fixa min=max=tamanho, mas
  não bastou. Preciso de: print da janela mini + `xdotool
  getwindowgeometry` dela ativa, pra saber se é piso do WM, sombra CSD ou
  conteúdo não preenchendo.

## Depois (maior)

- **Botão "check for updates" no topo.** Plugin updater do Tauri: par de
  chaves de assinatura (usuário gera), `latest.json` por release no CI,
  botão → baixa e instala. ~1h.
