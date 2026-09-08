# Yasmine

Player de música para PC (Rust) e Android, com sync direto por LAN — sem
conta, sem nuvem, sem servidor. O usuário aponta uma pasta; para sincronizar,
aponta o outro device.

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
| `crates/pc-app` | App desktop (`egui`/`eframe`). |
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
nunca entra na RAM.

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

Direção visual: software de áudio profissional, não app de streaming. Preto
quase absoluto, cantos retos, zero sombra, réguas de 1px, linhas de 22px
(~25 faixas visíveis sem rolar), mono nos números para as colunas alinharem.

O roxo/azul da identidade entra como **acento único**, em exatamente dois
lugares: a marca de 2px na faixa tocando e o preenchimento da barra de
progresso. Gradiente roxo espalhado é o que faz uma interface parecer
genérica; um acento contido faz o oposto.

A lista não mostra capa, de propósito: 25 miniaturas subindo e descendo a cada
rolagem custariam textura à toa, e a densidade é o ponto. A capa aparece na
barra do player e no modo compacto.

A sidebar de playlists segue a mesma linguagem: linha plana, marca de acento
de 2px em quem está ativo — biblioteca ou uma playlist, nunca as duas.

O controle de volume mestre é uma barra fina igual à de progresso, mas em
cinza neutro, não no acento — o acento continua significando uma coisa só
("é isto que está tocando"), e volume não é isso. Não aparece no modo
compacto: a largura de 340px já está no limite só com capa, texto e
transporte.

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
| botão direito numa faixa | adicionar a playlist, mover, remover |

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
cargo run --release -p player-pc -- ./testdata/lib50k
```
