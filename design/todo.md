# Ajustes pendentes da UI

Tudo do último lote foi feito (v0.5.3):

- Equalizer para quando pausa.
- Mutar clicando no ícone de volume (`VolumeX` quando mudo).
- Bolinha na barra de volume no hover.
- Título/álbum do now-playing usa mais largura (`.np`/`.right` flex 1 1 0).
- Modo compacto: os WMs (xfwm4/KWin) não deixam a janela abaixo de ~200px
  de altura — em vez de tira fina com vão preto, o MiniBar virou um player
  compacto de verdade (capa 56px, faixa, progresso, transporte) que
  preenche o espaço.

## Depois (maior)

- **Botão "check for updates" no topo.** Plugin updater do Tauri: par de
  chaves de assinatura (usuário gera), `latest.json` por release no CI,
  botão → baixa e instala. ~1h.
