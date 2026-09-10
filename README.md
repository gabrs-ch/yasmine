# Yasmine

Player de música para PC (Rust) e Android, com sync direto por LAN — sem
conta, sem nuvem, sem servidor. O usuário aponta uma pasta; para sincronizar,
aponta o outro device.

## Instalação

**[github.com/gabrs-ch/yasmine/releases/latest](https://github.com/gabrs-ch/yasmine/releases/latest)**

- **Windows**: baixe o `Yasmine_*-setup.exe` e rode. Instala só pro usuário
  atual (sem admin) e registra o "Abrir com" pros formatos de áudio. O
  Windows pode avisar "O Windows protegeu seu PC" na primeira vez (não é
  assinado) — "Mais informações" → "Executar assim mesmo". A interface usa
  o **WebView2**, que o Windows 11 já traz; nas máquinas sem ele o
  instalador baixa e adiciona na hora (precisa de internet nessa primeira
  instalação).
- **Linux**: baixe o `Yasmine_*.AppImage`, `chmod +x` e rode — um arquivo
  só, portátil, sem root; apagar é apagar o arquivo. Já traz o **WebKitGTK**
  embutido, então não depende do que a distro tem. Quem prefere pacote
  nativo: o `.deb` (Debian/Ubuntu) declara a dependência e o apt resolve.

A única coisa que fica no sistema é a pasta de dados do player (índice da
biblioteca e cache de capa, nos diretórios padrão).

O "Abrir com" do gerenciador de arquivos já vem configurado pelo instalador
(`.AppImage`/`.deb`/`setup.exe`): abrir um ou vários arquivos de áudio de
uma vez aponta a biblioteca pra pasta deles e toca a partir do primeiro.

### Compilar você mesmo

Precisa de toolchain Rust estável ([rustup.rs](https://rustup.rs)),
Node 20+ e, no Linux, os `-dev` do WebKitGTK
(`libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev librsvg2-dev`).

```
cargo install tauri-cli --version "^2.0"
npm --prefix crates/pc-app/ui ci
cargo tauri dev     --config crates/pc-app/tauri.conf.json   # rodar
cargo tauri build   --config crates/pc-app/tauri.conf.json   # empacotar
```

(ou `cd crates/pc-app && cargo tauri dev` / `build`.)

## Princípios

1. **Otimização é o critério de desempate** em toda decisão técnica.
2. **Nenhuma otimização vira configuração.** Não existe "modo performance",
   ajuste de buffer nem limpeza manual de cache. O default é o melhor que a
   gente consegue, e é invisível.
3. Cada fase entrega algo usável.

## Crates

| Crate | Papel |
|---|---|
| `crates/core` | Modelo, schema, índice. Único crate compartilhado entre PC e Android. |
| `crates/audio` | Decode e playback no PC (`cpal` + `symphonia`). Não vai pro Android. |
| `crates/sync` | Pareamento, mDNS, canal Noise, sync. Isolado: é o único que fala com a rede. |
| `crates/pc-app` | App desktop: back Rust (Tauri 2) + front web em `ui/` (React/TS). |
| `crates/android-ffi` | Bindings Kotlin via uniffi. |
| `tools/libgen` | Gerador de biblioteca sintética para medição. |

O binário final chama-se `yasmine`; os crates internos mantêm o prefixo
`player-*` — são identificadores de implementação, não aparecem pro usuário.

## Decisões fechadas

**`egui`/`eframe` com backend `glow`, não `wgpu`.** Criar o contexto GL é mais
rápido e traz menos dependência — aparece direto no cold start. Vêm junto:
repaint reativo (só em evento) e, tocando, repaint limitado a ~4 Hz. O padrão
de redesenhar a 60 fps é metade do custo de CPU de um player parado.

**`cpal` + `symphonia` direto, sem `rodio`.** O rodio reamostra sempre que a
taxa do device não bate com a do arquivo, com interpolação linear. Indo direto
no cpal dá pra abrir o device *na taxa do arquivo* e eliminar a reamostragem.
E o gapless sai quase de graça pré-decodificando no mesmo ring buffer.

**SQLite, definitivo.** WAL + `synchronous=NORMAL` + mmap. 50k faixas não chega
perto de ser gargalo; o custo real está no I/O do scan.

**FTS5 com `remove_diacritics 2`.** Uma decisão resolve busca instantânea *e*
acento: "jose" acha "José" sem coluna normalizada extra.

**A lista nunca carrega a biblioteca na RAM.** A view é um `Vec<TrackId>`
ordenado (400 KB para 50k faixas) e a UI busca só as linhas visíveis — nunca
`LIMIT/OFFSET`, que é O(n).

**Capa deduplicada por BLAKE3, miniaturas em disco.** É o maior consumidor de
memória de um player: a mesma arte se repete em toda faixa do álbum. Original
nunca entra na RAM. Duas miniaturas por capa (96px pra linha da lista, 512px
pra capa em destaque), reamostradas com Lanczos3 — a redução roda uma vez
por capa, dentro do worker paralelo do scan que já é I/O bound, então o
custo a mais não aparece no relógio, mas a diferença aparece na tela.

**Arquivo manda na metadata; o DB é cache derivado.** Estado do usuário
(playlist, plays, rating) vive à parte, com timestamp por campo. É o que
barateia o sync: áudio vira transferência endereçada por conteúdo, sem
conflito possível, e só o estado do usuário precisa de merge — LWW por campo,
e `MAX` por (faixa, device) para contagem de plays, que somada não perde play
nenhum.

**No Android o ExoPlayer toca e o Rust indexa.** Notificação, tela de bloqueio,
Bluetooth e Android Auto saem prontos e testados. O Rust é o **único** escritor
do arquivo SQLite; o Kotlin só consulta via FFI.

**Índice fracionário pra posição de item de playlist.** Ver
[`fracidx.rs`](crates/core/src/fracidx.rs). A posição é uma string, não um
número: mover um item escreve uma linha em vez de renumerar a playlist
inteira, e dois devices reordenando ao mesmo tempo não colidem quando o sync
chegar. A chave tem parte inteira (a primeira letra codifica o tamanho), o que
mantém 50 000 acréscimos em sequência em 4 bytes por chave — sem isso, seria
bissecção pura e a chave cresceria a cada inserção no mesmo ponto.

**Item de playlist aponta pelo hash da faixa, não pelo id.** `track.id` é
autoincrement local: o mesmo arquivo tem id diferente em cada device. Um item
sem faixa local correspondente vira buraco na lista, não desaparece — é a
faixa que ainda não chegou por sync.

**Sem ícone de bandeja no Linux.** `tray-icon` (a opção óbvia em Rust) puxa
GTK3 no Linux via `libxdo`/`gtk`, contra a decisão de manter o binário
enxuto. Existe `ksni` (implementação pura em D-Bus, sem GTK) como alternativa
mais tarde; por ora, o modo compacto (`Ctrl+M`) cobre o caso de uso de "ficar
tocando ocupando pouco espaço" sem a dependência.

**Nivelador de volume por RMS, não EBU R128/ReplayGain de verdade.** A medida
"correta" de volume percebido usa filtro de ponderação-K e gating de trechos
silenciosos (ITU-R BS.1770) — implementar esse filtro do zero é boa parte do
trabalho de uma biblioteca de áudio inteira, para um ganho de precisão que não
muda a decisão prática. RMS do sinal decodificado já resolve "essa faixa é
gravada mais baixo que as outras" na esmagadora maioria dos casos, com uma
fração do código. O ganho final nunca passa de `1 / pico` medido na faixa —
sem isso, uma faixa gravada baixo mas com transientes agudos receberia o
ganho cheio do RMS e estouraria 0 dBFS nesses trechos. Ver
[`loudness.rs`](crates/audio/src/loudness.rs).

**Medição em segundo plano roda sequencial, não em paralelo entre núcleos.**
Ao contrário do hash (BLAKE3, ~1–3 GB/s, insignificante mesmo saturando todos
os núcleos), decodificar áudio é caro, e essa tarefa pode rodar por minutos
enquanto o usuário ouve música ao mesmo tempo. Um fan-out em todos os núcleos
competiria com a decodificação da faixa que está tocando *agora*, e um
glitch audível custa muito mais que terminar de nivelar a biblioteca alguns
minutos mais cedo. Faixa nova toca sem nivelamento até a tarefa de fundo
chegar nela — nunca espera a medição para começar a tocar.

**Volume mestre é a única preferência que o app lembra entre sessões.** Não é
"configuração" no sentido que este projeto evita — é o mesmo tipo de memória
que a pasta escolhida já tinha: básico o bastante para não contar como opção
exposta, só como o app lembrando o que você já tinha ajustado.

## Fases

- [x] **0 — Fundamentos.** Workspace, schema, clippy/fmt no CI, gerador de biblioteca.
- [x] **1 — Player PC.** Pasta → scan → índice → tocar. Lista, play/pause/next/prev, busca, seek.
- [x] **2 — Polimento PC.** Fila, playlists, shuffle/repeat, atalhos, modo compacto, `notify`, nivelador de volume, profiling.
- [ ] **3 — Android standalone.** Compose + `core` via uniffi.
- [ ] **4 — Sync na LAN.** QR → mDNS → Noise → diff por hash.
- [ ] **5 — Refinamento.** Profiling real, biblioteca grande, onboarding.

[`docs/contexto-android.md`](docs/contexto-android.md) é o handoff pra
quem for escrever o Android e o sync local: arquitetura, modelo de dados,
o que se reaproveita do `core`, e o desenho do pareamento por QR + download
das músicas do PC pro celular, com os pontos ainda em aberto.

## Medições

Numa VM de 4 núcleos e 3,8 GB, com renderização por software (sem GPU),
biblioteca sintética de 50 000 faixas em 4 998 álbuns:

| | |
|---|---|
| Primeiro scan | 1,7 s · 75 MB de pico · 4 998 capas decodificadas |
| Rescan sem mudanças | 0,19 s · nenhum arquivo aberto |
| Índice em disco | 20 MB |
| Montar a lista ordenada | 22,9 ms (390 KB de ids) |
| Buscar enquanto digita | 2,4 ms (2 044 resultados) |
| Janela visível da lista | 0,1 ms (40 linhas) |
| Janela aberta | ~530 ms |
| **CPU com a janela aberta e parada** | **0%** |
| Playback | 0 underruns · 0,02 s de CPU em 3,3 s |

A capa sintética é pequena; com capa real de 1000×1000 cada álbum novo custa
~5,8 ms de decode e resize, o que somaria ~8 s (em 4 threads) ao *primeiro*
scan de 5 000 álbuns. Rescans não pagam nada disso.

Do RSS de 143 MB do app, 67 MB são o `libLLVM` do llvmpipe — o rasterizador
OpenGL por software desta VM, que não existe numa máquina com driver de GPU.

### Fase 2 — fila, playlists, modo compacto, vigia de arquivos

Mesma biblioteca de 50 000 faixas, mais uma playlist com todas elas — o caso
que estressa índice fracionário e hash sob demanda de uma vez:

| | |
|---|---|
| Criar playlist com 50 000 faixas (hash de tudo, 1ª vez) | 410 ms |
| Mesma operação, hash já calculado | 147 ms |
| Ler a playlist de volta (50 000 itens) | 32,8 ms |
| Listar playlists existentes | 2,6 ms |
| RSS do app com biblioteca + playlist de 50 000 carregadas | 157 MB |
| Rescan ao reabrir (conteúdo intacto) | 0,4 s |

O hash é sobre arquivos sintéticos de ~16 KB; num álbum de verdade (3–10 MB
por faixa) o custo desloca de CPU pra I/O de disco — BLAKE3 satura a leitura
bem antes de virar o gargalo.

**Descoberto testando a UI de verdade, não só a biblioteca `core`:** o modo
compacto desenhava certinho para 340×112, mas a janela do SO ficava presa em
620×380 — o `min_inner_size` configurado na abertura não tinha sido relaxado
antes do pedido de encolher. E `xdotool getwindowgeometry`, usado nos scripts
de teste desta sessão, reporta a posição do frame decorado pelo gerenciador
de janelas, não da área cliente — um offset de ~24px que fazia clique em
alvo pequeno (o botão "+" de nova playlist) errar sistematicamente, enquanto
alvos grandes (linha da lista) toleravam o erro por sorte. `xwininfo -id`
resolve a reparentagem corretamente e virou o método padrão de ali em diante.

## Interface

Direção visual: Apple Music, só que escuro. Substitui a direção original
("software de áudio profissional", cantos retos em tudo) — densidade extrema
sem capa nenhuma lia como planilha, não como player, e essa foi a queixa que
motivou a virada. Preto quase absoluto continua, mas cantos arredondados
(`theme::RADIUS`, um número só, usado em todo lugar) viraram a regra, não
exceção pontual; linhas mais altas (44px, contra 22px antes) dão espaço pra
capa respirar; réguas de 1px e mono nos números continuam.

O roxo/azul da identidade continua **acento único** — a marca de 2px na
faixa tocando e o preenchimento da barra de progresso — mas parou de ser a
única coisa arredondada da interface.

A capa aparece em toda parte agora: miniatura arredondada em cada linha da
lista (carregada pelo `ArtLoader` já existente, sem custo extra — o cache de
texturas já era dimensionado pra isso), no player, no modo compacto, e —
novo — pode ser escolhida manualmente: botão direito numa faixa → "Escolher
capa do álbum…" abre um seletor nativo de arquivo, decodifica pelo mesmo
`ArtCache` do scan (dedup por hash, miniaturas geradas do mesmo jeito) e
grava no álbum inteiro, não só na faixa clicada — é o álbum que carrega a
capa no índice, então uma escolha vale pra toda faixa dele.

A sidebar de playlists segue a mesma linguagem: linha alta, destaque recuado
e arredondado, marca de acento de 2px em quem está ativo — biblioteca, uma
playlist ou um artista, nunca mais de uma.

**Ver só um artista**: botão direito numa faixa → "Ver só faixas de X". A
lista passa a mostrar só as faixas dele (como intérprete da faixa ou como
artista do álbum — uma coletânea onde ele aparece "feat." ainda conta), e a
sidebar mostra o nome recuado sob BIBLIOTECA, marcado como a fonte atual.
Clicar em BIBLIOTECA volta pra biblioteca inteira. Não tem lista de artistas
navegável na sidebar: com centenas de artistas viraria uma coluna infinita,
e o caminho "estou olhando uma faixa, quero mais desse artista" cobre o
essencial sem ocupar espaço fixo. Reusa `Source` (agora com um braço
`Artist`) e todo o resto — busca, ordenação, fila — funciona dentro do
recorte.

Uma playlist pode ser **vinculada a uma pasta** (botão direito → "Vincular
pasta…"): toda faixa que está, ou vier a entrar, dentro dela passa a fazer
parte da playlist sozinha, sem arrastar uma por uma. Guardado por (raiz,
prefixo relativo) — não por caminho absoluto — pelo mesmo motivo de
`track.rel_path`: mover a pasta de raiz inteira de lugar não invalida o
vínculo. A sincronização roda depois de cada scan (manual ou pelo vigia de
arquivos) e é idempotente: só acrescenta o que ainda não está lá, então um
rescan repetido nunca duplica item.

O controle de volume mestre e a barra de progresso são pílula com bolinha
arrastável, no espírito do slider do Apple Music — um controle contínuo se
lê melhor como objeto físico do que como dado tabular. Cor neutra no volume
(não é "o que está tocando"), acento na barra de progresso (é). A bolinha da
barra de progresso só aparece em hover/arraste, pra não pesar visualmente
numa barra que fica sempre visível durante o playback inteiro. A área de
clique/arraste dos dois é bem mais alta (20px) que o traço visual (4px):
mirar exatamente numa linha fina é chato, e o alvo generoso não muda como a
barra parece, só como ela responde. O preenchimento dos dois é gradiente, não
cor chapada — faixas verticais finas com cor interpolada (`egui::Painter` não
tem gradiente nativo), mesmo matiz nas duas pontas pra continuar sendo UM
acento, só com luminosidade variando; no volume o gradiente fica no cinza,
nunca no acento, porque volume não é "o que está tocando".

A janela é sem decoração nativa (`with_decorations(false)`). No Linux, quem
desenha o cabeçalho de uma janela é o gerenciador de janelas do usuário — no
XFCE, um cabeçalho cinza claro genérico, colado direto num conteúdo quase
preto sem nenhuma relação com ele. Era a costura mais feia da janela inteira,
e nenhum ajuste de cor dentro do app resolvia, porque o app não desenhava
aquela barra. Agora desenha: a própria barra de comando (`top_bar`) também é
a barra de título — arrasta em área livre, duplo clique maximiza/restaura,
os botões de minimizar/maximizar/fechar são vetoriais, no mesmo traço do
resto da interface (fechar fica vermelho, a única concessão fora da paleta
de acento único — convenção forte demais pra abrir mão). O modo compacto
ganhou o mesmo arraste, senão perderia a única razão de existir ("fica num
canto da tela") sem ter mais barra nativa pra arrastar — e, pelo mesmo
motivo, no modo compacto a janela fica sempre por cima das outras
(`WindowLevel::AlwaysOnTop`), voltando ao normal ao sair. Resultado:
a janela fica com a mesma cara em qualquer ambiente — XFCE, GNOME, KDE — em
vez de herdar o que cada um decidir desenhar.

Duas coisas que a decoração nativa dava de graça e precisaram ser refeitas à
mão: a margem entre o conteúdo e a quina da janela (a decoração *era* essa
margem — sem ela, o botão de escolher pasta ficava colado no canto esquerdo
e o de fechar no direito, ambos com 4px de folga contra uma quina totalmente
reta, sem nenhum arredondamento do sistema pra suavizar) e o traço de 1px em
volta da janela inteira (sem ele, o retângulo se perdia contra o fundo da
área de trabalho por trás). Os dois voltaram: 12px de respiro nos cantos da
barra de comando, e um `rect_stroke` na cor `RULE` desenhado numa camada de
primeiro plano, por cima de tudo, já que não pertence a painel nenhum.

A capa em destaque (player normal e modo compacto) ganhou um halo suave —
anéis concêntricos do acento com alfa decrescente atrás do quadrado
arredondado, a aproximação vetorial de um desfoque que o `Painter` não tem.
Título da faixa tocando também cresceu (mesmo `TextStyle::Heading` que o
resto da interface já reservava e não usava) e o título de cada linha da
lista ganhou peso — sem fonte bold embutida no binário, o texto é desenhado
duas vezes com um deslocamento de 0,4px, o mesmo truque de sempre pra
contraste de peso sem arquivo extra. Um fio de 1px separa cada região da
janela da vizinha (topo/lista, sidebar/lista, e um realce quase transparente
no topo do player, como se ele flutuasse à frente) — antes a única
articulação entre elas era a diferença de tom entre `PANEL` e `BG`.

"Pasta…", a única ação possível antes de escolher uma biblioteca, é botão de
ação primária (preenchido no acento) nesse momento específico — e só nesse:
com a biblioteca carregada, "trocar de pasta" e "reescanear" viram ícones
(pasta e setas circulares), com o que fazem no tooltip. Botão de texto na
barra só quando o texto É a informação (a primeira escolha, numa tela vazia
— e a tela de boas-vindas repete essa oferta grande, com o halo atrás da
marca). Depois disso, ação ocasional não precisa de rótulo ocupando a
barra. O campo de busca ganhou uma lupa à esquerda, mesma fonte de ícone do
resto.

**Todo ícone da interface** — transporte, shuffle/repeat/modo compacto, "+"
de nova playlist, lupa da busca, pasta e reescanear, os três controles de
janela — vem da mesma fonte: [Lucide](https://lucide.dev) (ISC, `assets/lucide.ttf`,
`assets/LUCIDE-LICENSE.txt`), embutida no binário como qualquer outro
asset. Começou como forma vetorial desenhada à mão (`Painter::line_segment`,
`convex_polygon`) pela mesma razão de sempre — traço nítido garantido, sem
depender de a fonte do sistema ter o símbolo certo — mas ícone bem desenhado
é ofício de quem faz isso o dia inteiro, não de reinventar cada forma em
coordenada de pixel. A fonte resolve os dois ao mesmo tempo: continua
embutida (mesma garantia de traço, zero dependência do ambiente) e o
desenho em si é o de gente que projeta ícone pra viver. Registrada como família
`FontFamily::Name` própria (`theme::icon_family`), nunca entra nas famílias
de texto normal — só quem chama `theme::icon()` explicitamente a enxerga.

Os três controles de janela (minimizar/maximizar/fechar) merecem nota à
parte: o Lucide tem ícones dedicados pra essas duas primeiras ações (setas
de canto pra dentro/fora), visualmente mais originais que traço e quadrado
— mas na prática lêem como "entrar/sair de tela cheia", não "minimizar pra
barra de tarefas"/"maximizar". Testado, achado confuso, revertido pro
traço e quadrado simples: a convenção universal existe por um motivo, e
reconhecível vale mais que original numa ação que o usuário precisa
identificar sem pensar. O fundo de hover dos três é círculo, não o
cantos-arredondados do resto dos botões — um "controle de janela" lê melhor
como forma fechada em si, no espírito dos três pontinhos do macOS, do que
como mais um botão retangular na fileira.

**Texto**: [IBM Plex Sans](https://www.ibm.com/plex/) (OFL,
`assets/IBMPlexSans-{Regular,SemiBold}.ttf`) no lugar da fonte padrão que o
`egui` já traz embutida, e [JetBrains Mono](https://www.jetbrains.com/lp/mono/)
(OFL, build "NL" — sem ligadura de programação, que não faz sentido pra
exibir duração de faixa) no lugar do mono padrão. A fonte padrão existe pra
rodar em qualquer lugar sem asset nenhum; "roda em qualquer lugar" e
"bonita" são objetivos diferentes, e só dá pra ter os dois embutindo a
própria. Plex é humanista, não a neogrotesca genérica — tem calor e desenho
próprio sem custar legibilidade em tamanho de UI (a rodada anterior com
Inter lia como "software corporativo"). As duas entram com prioridade
`Highest` nas famílias `Proportional`/`Monospace` — não substituem o que o
`egui` já registrou ali, ficam na frente; o que sobra (emoji, por exemplo)
continua caindo nas fontes padrão como reserva, em vez de sumir. O título da
faixa — na lista e na barra do player — usa peso de verdade (`theme::strong`,
a família SemiBold), não o texto desenhado duas vezes com deslocamento que
fingia negrito antes de ter fonte com peso no binário.

**A marca do Yasmine** é uma nota musical brotando folhas, roxa com
gradiente, num quadrado arredondado claro — arte fornecida pelo usuário, no
formato de ícone de app.
[`assets/icon-256.png`](crates/pc-app/assets/icon-256.png) é o arquivo único
— janela, atalho da área de trabalho e tela de boas-vindas do app carregam
o mesmo PNG (com transparência em volta do quadrado) como textura, em vez de
cada lugar ter sua própria cópia ou reimplementação.

## Atalhos

| | |
|---|---|
| `Espaço` | tocar / pausar |
| `↑` `↓` | mover a seleção |
| `←` `→` | faixa anterior / próxima |
| `Enter` | tocar a seleção |
| `Ctrl+F` | ir para a busca |
| `S` | shuffle |
| `R` | repetir (desligado → tudo → uma → desligado) |
| `Ctrl+M` | modo compacto |
| duplo clique | tocar a faixa |
| botão direito numa faixa | ver só faixas do artista, adicionar a playlist, mover, remover |
| botão direito numa playlist | renomear, apagar, vincular/desvincular pasta |

## Desenvolvimento

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

Biblioteca sintética para medir scan, índice e memória — determinística, a
mesma seed dá o mesmo corpus byte a byte:

```bash
cargo run --release -p libgen -- --out ./testdata/lib50k --tracks 50000
```

50k faixas em ~2s, ocupando 1,2 GB. Como regenerar é barato, o corpus grande
não fica versionado nem guardado — `testdata/` está no `.gitignore`.

Medir o scan e as consultas contra uma pasta de verdade (rodar duas vezes: a
segunda passada é o que mostra se o caminho incremental está funcionando):

```bash
cargo run --release -p player-core --example scan -- ./testdata/lib50k /tmp/lib.db
```

Medir uma playlist grande (hash de tudo de uma vez, e leitura de volta):

```bash
cargo run --release -p player-core --example playlist_bench -- ./testdata/lib50k
```

Rodar o player apontado numa pasta:

```bash
cargo tauri dev --config crates/pc-app/tauri.conf.json -- ./testdata/lib50k
```
