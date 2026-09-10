# Ajustes pendentes da UI

Tudo do último lote foi feito (v0.5.3 / v0.5.4):

- Equalizer para quando pausa.
- Mutar clicando no ícone de volume (`VolumeX` quando mudo).
- Bolinha na barra de volume no hover.
- Título/álbum do now-playing usa mais largura (`.np`/`.right` flex 1 1 0).
- Modo compacto (v0.5.4): virou uma **faixa larga e baixa** — capa à
  esquerda, título + progresso no meio, transporte à direita. Pede
  560×104. Os WMs (xfwm4/KWin) impõem um piso de altura (~200px); quando
  isso acontece o conteúdo centraliza na vertical e não sobra vão preto,
  mas o layout continua horizontal (não mais um quadrado alto).

## Depois (maior)

- **Botão "check for updates" no topo.** Plugin updater do Tauri: par de
  chaves de assinatura (usuário gera), `latest.json` por release no CI,
  botão → baixa e instala. ~1h.
