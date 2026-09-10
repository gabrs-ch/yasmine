# Ajustes pendentes da UI

Tudo do último lote foi feito (v0.5.3 / v0.5.4):

- Equalizer para quando pausa.
- Mutar clicando no ícone de volume (`VolumeX` quando mudo).
- Bolinha na barra de volume no hover.
- Título/álbum do now-playing usa mais largura (`.np`/`.right` flex 1 1 0).
- Modo compacto (v0.5.4 → v0.5.5): virou uma **faixa larga e baixa** —
  capa à esquerda, título + progresso no meio, transporte à direita,
  560×116. O piso de ~200px de altura vinha do `setResizable(false)`: no
  GTK isso faz a janela ignorar as geometry hints e travar no tamanho
  "natural" do conteúdo. Solução (v0.5.5): não mexer em resizable; fixar
  o tamanho com `minSize == maxSize`. Agora a janela sai exatamente em
  560×116 (verificado na VM com xfwm4).

## Depois (maior)

- **Botão "check for updates" no topo.** Plugin updater do Tauri: par de
  chaves de assinatura (usuário gera), `latest.json` por release no CI,
  botão → baixa e instala. ~1h.
