# Ajustes pendentes da UI (pós-v0.5.2)

Feedback do usuário testando no Fedora/KDE. Ainda não feito.

1. **Equalizer da faixa tocando: parar a animação quando pausado.** As
   barrinhas ao lado da faixa representam "tocando", não "selecionada" — hoje
   elas ficam animando mesmo com o playback em pausa.
   → `MainPane.tsx` / `.eq` em `app.css`: `animation-play-state: paused`
   quando `playback.playing === false`.

2. **Mutar clicando no ícone de volume.** O ícone vira `<button>` → alterna
   mudo (guarda o volume anterior). O ícone muda pra indicar mudo
   (`VolumeX` do lucide) e volta ao restaurar.
   → `PlayerBar.tsx` + `store` (`toggleMute`, `preMuteVolume`).

3. **Modo compacto ainda deixa a janela "grossa".** O v0.5.2 já sai de
   fullscreen/maximizado antes e fixa min=max, mas no KWin do usuário ainda
   fica com bandas grossas. Precisa de screenshot + `xdotool
   getwindowgeometry` da janela mini pra diagnosticar (piso do WM? sombra
   CSD? conteúdo não preenchendo?).

4. **Barra de volume igual à de progresso.** Bolinha redonda ao passar o
   mouse, pra facilitar o clique.
   → passar `knob` pro `<Scrubber>` do volume e CSS `.vol-line:hover
   .track-knob`.
