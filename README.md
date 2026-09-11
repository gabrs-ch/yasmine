# Yasmine

Player de música local para PC e Android, com sync direto por LAN — sem
conta, sem nuvem, sem servidor. Para quem compra a música e quer ouvir nos
dois aparelhos sem pedir licença pra ninguém.

## Por que existe

O streaming resolveu o acesso e quebrou a remuneração. O pagamento por
execução concentra a receita no topo do catálogo: quem não tem escala
recebe frações de centavo por play, e serviços passaram a desmonetizar
faixas abaixo de um piso anual de execuções. Comprar um álbum direto —
Bandcamp, a loja do selo, um Pix depois do show — entrega numa compra o
que levaria milhares de execuções pra render.

Só que quem compra fica com o pior dos dois mundos: um punhado de arquivos
numa pasta e nenhum jeito decente de ouvir. Os players locais ou pararam no
tempo, ou são um gerenciador de arquivos com botão de play, ou querem que
você suba a coleção pro servidor deles — o que recria exatamente a
dependência de que você tentou sair.

E o plano gratuito dos streamings é desenhado pra incomodar: anúncio,
ordem de reprodução limitada, qualidade reduzida, sem offline.

Yasmine é a outra ponta disso. Você compra, o arquivo é seu, e o player
trata esse caso como o normal em vez de exceção. Aponta a pasta no PC,
mostra um QR pro celular, e a biblioteca inteira — com playlists, contagem
de plays e rating — fica nos dois lados. Nada expira, nada precisa de
conexão depois que baixou, e não existe servidor no meio pra sair do ar.

## Escopo

**Faz:** indexa uma pasta, toca, busca, playlists, nivela volume entre
faixas de origens diferentes, e sincroniza PC → celular pela rede local.

**Não faz:** catálogo, recomendação, loja, conta, scrobble pra serviço
nenhum. Não baixa música de lugar nenhum — a música entra pela pasta que
você apontou, vinda de onde você comprou.

## Instalação

**[github.com/gabrs-ch/yasmine/releases/latest](https://github.com/gabrs-ch/yasmine/releases/latest)**

**Windows** — duas opções:

- `Yasmine_*_windows_portable.zip` — extraia e dê duplo clique no
  `yasmine.exe`. Portátil, não instala nada; apagar é apagar a pasta.
- `Yasmine_*-setup.exe` — instala só pro usuário atual (sem admin) e
  registra o "Abrir com" pros formatos de áudio.

O Windows pode avisar "O Windows protegeu seu PC" na primeira vez (não é
assinado) — "Mais informações" → "Executar assim mesmo". A interface usa o
**WebView2**, que o Windows 11 já traz; nas raras máquinas sem ele o
`setup.exe` baixa e adiciona na hora (o `.zip` portátil só roda se o WebView2
já estiver presente).

**Linux** — o pacote nativo é o caminho confiável: usa o **WebKitGTK** da
própria distro (o gerenciador resolve a dependência) e o "Abrir com" já vem
configurado.

- Debian/Ubuntu/Mint/Pop: `sudo apt install ./Yasmine_*_amd64.deb`
- Fedora/Nobara/RHEL/openSUSE: `sudo dnf install ./Yasmine-*.x86_64.rpm`
- `Yasmine_*.AppImage` (um arquivo só, sem root) também existe, com o
  WebKitGTK embutido — mas a mistura de libs empacotadas com as do host faz
  o `WebKitWebProcess` abortar em distros fora da família Ubuntu (visto no
  Fedora 44). Se acontecer, use o `.deb`/`.rpm`.

**Android** — `Yasmine_*_android.apk` na mesma página. Universal, instala
direto. Detalhes em [`docs/android.md`](docs/android.md).

A única coisa que fica no sistema é a pasta de dados do player (índice da
biblioteca e cache de capa, nos diretórios padrão).

O "Abrir com" do gerenciador de arquivos abre um ou vários arquivos de áudio
de uma vez, aponta a biblioteca pra pasta deles e toca a partir do primeiro.

### Compilar

Toolchain Rust estável ([rustup.rs](https://rustup.rs)), Node 20+ e, no
Linux, os `-dev` do WebKitGTK (`libwebkit2gtk-4.1-dev libgtk-3-dev
libsoup-3.0-dev librsvg2-dev`).

```sh
cargo install tauri-cli --version "^2.0"
npm --prefix crates/pc-app/ui ci
cargo tauri dev   --config crates/pc-app/tauri.conf.json   # rodar
cargo tauri build --config crates/pc-app/tauri.conf.json   # empacotar
```

O APK é outro caminho — `scripts/build-apk.sh`, ver
[`docs/android.md`](docs/android.md).

## Sync com o celular

Você comprou uma vez; ouvir no celular não devia custar assinatura nem
upload. Botão do telefone na barra do topo abre um painel com um QR code. O
app Android lê, e a biblioteca inteira do PC — arquivos, playlists, plays,
rating — desce pro celular. Depois disso o celular tem biblioteca própria e
toca offline, com o PC desligado.

O canal é Noise sobre TCP na LAN; o QR carrega a chave pública do PC, que é
a credencial. O sync é **pull**: o PC responde a pedidos e nunca é
modificado pelo celular.

→ **[`docs/sync.md`](docs/sync.md)** descreve o recurso ponta a ponta:
modelo de confiança, as duas camadas de dados, merge, garantias e limites.
O formato no fio está em [`docs/protocolo-sync.md`](docs/protocolo-sync.md).

## Princípios

1. **O arquivo é do usuário, e o app é descartável.** A metadata que vale é
   a que está no arquivo; o banco é cache derivado que dá pra jogar fora e
   reconstruir. Desinstalar não leva a coleção junto, e nenhuma decisão do
   projeto pode depender de o Yasmine continuar existindo.
2. **Otimização é o critério de desempate** em toda decisão técnica.
3. **Nenhuma otimização vira configuração.** Não existe "modo performance",
   ajuste de buffer nem limpeza manual de cache. O default é o melhor que a
   gente consegue, e é invisível.
4. Cada fase entrega algo usável.

## Mapa do repositório

| Caminho | Papel |
|---|---|
| `crates/core` | Modelo, schema, índice, scan, consultas. Compartilhado entre PC e Android. |
| `crates/audio` | Decode e playback no PC (`cpal` + `symphonia`). Não vai pro Android. |
| `crates/sync` | Pareamento por QR, mDNS, canal Noise, transferência, merge. Único crate que fala com a rede. |
| `crates/sync-host` | `yasmine-sync-host`: serve a biblioteca sem abrir a UI. |
| `crates/pc-app` | App desktop: back Rust (Tauri 2) + front React/TS em `ui/`. |
| `crates/android-ffi` | Ponte uniffi (`YasmineLibrary` + `Syncer`) → Kotlin. |
| `android/` | App Android (Compose + ExoPlayer + CameraX). |
| `tools/libgen` | Gerador de biblioteca sintética para medição. |
| `docs/` | [arquitetura](docs/arquitetura.md) · [sync](docs/sync.md) · [protocolo](docs/protocolo-sync.md) · [android](docs/android.md) · [teste do sync](docs/teste-sync.md) |
| `design/` | Mockup aprovado e histórico da migração pro Tauri. |

## Decisões fechadas

**UI em Tauri 2 (webview do SO) + front web, não `egui`.** A primeira versão
foi `egui`/`eframe`: um binário, zero dependência de runtime, cold start
rápido. Mas o teto visual do modo imediato é baixo pra este trabalho —
`letter-spacing`, transição/animação, gradiente livre, layout que centra de
verdade, AA de texto. O Tauri renderiza o front (React/TS em
[`crates/pc-app/ui`](crates/pc-app/ui)) no webview que o SO já traz, então
CSS de verdade e o mockup vira código. Custo assumido: +RAM (~50 → ~150 MB),
cold start mais lento, e a dependência de runtime. Histórico e plano da
migração em [`design/`](design/).

**`cpal` + `symphonia` direto, sem `rodio`.** O rodio reamostra sempre que a
taxa do device não bate com a do arquivo, com interpolação linear. Indo
direto no cpal dá pra abrir o device *na taxa do arquivo* e eliminar a
reamostragem. E o gapless sai quase de graça pré-decodificando no mesmo ring
buffer.

**SQLite, definitivo.** WAL + `synchronous=NORMAL` + mmap. 50k faixas não
chega perto de ser gargalo; o custo real está no I/O do scan.

**FTS5 com `remove_diacritics 2`.** Uma decisão resolve busca instantânea *e*
acento: "jose" acha "José" sem coluna normalizada extra.

**A lista nunca carrega a biblioteca na RAM.** A view é um `Vec<TrackId>`
ordenado (400 KB para 50k faixas) e a UI busca só as linhas visíveis — nunca
`LIMIT/OFFSET`, que é O(n).

**Capa deduplicada por BLAKE3, miniaturas em disco.** É o maior consumidor
de memória de um player: a mesma arte se repete em toda faixa do álbum.
Original nunca entra na RAM. Duas miniaturas por capa (96px pra linha da
lista, 512px pra capa em destaque), reamostradas com Lanczos3 — a redução
roda uma vez por capa, dentro do worker paralelo do scan que já é I/O bound,
então o custo a mais não aparece no relógio.

**Arquivo manda na metadata; o DB é cache derivado.** Álbum comprado vem
com tag e capa decentes — a fonte da verdade é o arquivo, e apagar o banco e
reescanear devolve o mesmo estado. Estado do usuário (playlist, plays,
rating) vive à parte, com timestamp por campo, porque esse não dá pra
reconstruir. É também o que barateia o sync: áudio vira transferência
endereçada por conteúdo, sem conflito possível, e só o estado do usuário
precisa de merge.

**Item de playlist aponta pelo hash da faixa, não pelo id.** `track.id` é
autoincrement local: o mesmo arquivo tem id diferente em cada device. Um
item sem faixa local vira buraco na lista, não desaparece — é a faixa que
ainda não chegou por sync.

**Índice fracionário pra posição de item de playlist.** Ver
[`fracidx.rs`](crates/core/src/fracidx.rs). A posição é uma string, não um
número: mover um item escreve uma linha em vez de renumerar a playlist
inteira, e dois devices reordenando ao mesmo tempo não colidem no merge. A
chave tem parte inteira (a primeira letra codifica o tamanho), o que mantém
50 000 acréscimos em sequência em 4 bytes por chave.

**Apagar é marcar.** Playlist e item de playlist deixam túmulo
(`deleted`/`deleted_at`) em vez de sumir com a linha. Sem isso, "não tenho
essa linha" é indistinguível de "apaguei", e o sync reintroduz o que foi
removido.

**No Android o ExoPlayer toca e o Rust indexa.** Notificação, tela de
bloqueio, Bluetooth e Android Auto saem prontos e testados. O Rust é o
**único** escritor do arquivo SQLite; o Kotlin só consulta via FFI.

**Nivelador de volume por RMS, não EBU R128/ReplayGain de verdade.** A
medida "correta" de volume percebido usa filtro de ponderação-K e gating de
trechos silenciosos (ITU-R BS.1770) — implementar esse filtro do zero é boa
parte do trabalho de uma biblioteca de áudio inteira, para um ganho de
precisão que não muda a decisão prática. RMS do sinal decodificado já
resolve "essa faixa é gravada mais baixo que as outras" na esmagadora
maioria dos casos. O ganho final nunca passa de `1 / pico` medido na faixa —
sem isso, uma faixa gravada baixo mas com transientes agudos estouraria
0 dBFS. Ver [`loudness.rs`](crates/audio/src/loudness.rs).

**A medição de volume roda sequencial, não em paralelo entre núcleos.** Ao
contrário do hash (BLAKE3, ~1–3 GB/s, insignificante mesmo saturando os
núcleos), decodificar áudio é caro, e essa tarefa pode rodar por minutos
enquanto o usuário ouve música. Um fan-out competiria com a decodificação da
faixa que está tocando *agora*, e um glitch audível custa mais que terminar
de nivelar alguns minutos mais cedo. Faixa nova toca sem nivelamento até a
tarefa de fundo chegar nela.

**Volume mestre é a única preferência que o app lembra entre sessões.** Não
é "configuração" no sentido que este projeto evita — é o mesmo tipo de
memória que a pasta escolhida já tinha.

**Sem ícone de bandeja.** O modo compacto (`Ctrl+M`) — janela encolhida a
uma faixa larga, sempre no topo — cobre "ficar tocando ocupando pouco
espaço". Bandeja de verdade só se aparecer necessidade.

## Fases

- [x] **0 — Fundamentos.** Workspace, schema, clippy/fmt no CI, gerador de biblioteca.
- [x] **1 — Player PC.** Pasta → scan → índice → tocar. Lista, transporte, busca, seek.
- [x] **2 — Polimento PC.** Fila, playlists, shuffle/repeat, atalhos, modo compacto, `notify`, nivelador de volume.
- [x] **3 — Android.** Compose + ExoPlayer, `core` via uniffi.
- [x] **4 — Sync na LAN.** QR → Noise → diff por hash → merge. Ver [`docs/sync.md`](docs/sync.md).
- [ ] **5 — Refinamento.** Sync no sentido inverso, biblioteca grande, onboarding.

## Interface

Direção visual: Apple Music, só que escuro — preto quase absoluto, cantos
arredondados, linhas altas o bastante pra capa respirar, mono nos números.
Substitui a direção original ("software de áudio profissional", cantos
retos), que sem capa nenhuma lia como planilha, não como player. O
roxo/azul da identidade é **acento único**: a marca na faixa tocando e o
preenchimento da barra de progresso.

O mockup aprovado está versionado em
[`design/mockup.html`](design/mockup.html) e é a fonte da verdade de cada
cor, raio e medida. Os tokens vivem no `:root` de
[`app.css`](crates/pc-app/ui/src/styles/app.css); o app Android traduz os
mesmos valores pra `darkColorScheme`, então os dois frontes têm a mesma
paleta sem duplicar a decisão.

A janela não usa decoração nativa. No Linux quem desenha o cabeçalho é o
gerenciador de janelas do usuário — no XFCE, uma barra cinza clara genérica
colada num conteúdo quase preto. A barra de comando do app *é* a barra de
título: arrasta em área livre, e os botões de janela são desenhados junto
com o resto. Como borda nenhuma vem do sistema, o app desenha a própria
(`.win { border-radius }` sobre janela transparente) e oito faixas
invisíveis nas bordas chamam `startResizeDragging`, que é o que a decoração
nativa dava de graça.

**Ícones**: [Lucide](https://lucide.dev) (ISC), via `lucide-react` no
desktop e `material-icons-extended` no Android. Os quatro do transporte
(prev/next/play/pause) são SVG inline — são cheios e simples, e o
preenchimento do Lucide não bate com o desenho do mockup.

**Texto**: [IBM Plex Sans](https://www.ibm.com/plex/) (OFL) e
[JetBrains Mono](https://www.jetbrains.com/lp/mono/) (OFL, build "NL" — sem
ligadura de programação, que não faz sentido pra exibir duração de faixa),
embutidas nos dois apps. Plex é humanista, não a neogrotesca genérica: tem
desenho próprio sem custar legibilidade em tamanho de UI.

**A marca** é uma nota musical brotando folhas, roxa com gradiente, num
quadrado arredondado — arte do usuário.
[`assets/icon-source.png`](crates/pc-app/assets/icon-source.png) é o
arquivo único: janela, atalho, tela de boas-vindas e o ícone do Android
saem todos dele.

## Atalhos (desktop)

| | |
|---|---|
| `Espaço` | tocar / pausar |
| `←` `→` | voltar / avançar 5 s |
| `S` | shuffle |
| `R` | repetir (desligado → tudo → uma → desligado) |
| `Ctrl+M` | modo compacto |
| duplo clique | tocar a faixa |
| botão direito numa faixa | ver só faixas do artista, adicionar a playlist, mover, remover, escolher capa |
| botão direito numa playlist | renomear, apagar, foto, vincular/desvincular pasta |

No Android, toque longo abre o menu de contexto equivalente.

## Medições

Numa VM de 4 núcleos e 3,8 GB, com renderização por software (sem GPU),
biblioteca sintética de 50 000 faixas em 4 998 álbuns. Estes números são do
`player-core` e valem pros dois frontes:

| | |
|---|---|
| Primeiro scan | 1,7 s · 75 MB de pico · 4 998 capas decodificadas |
| Rescan sem mudanças | 0,19 s · nenhum arquivo aberto |
| Índice em disco | 20 MB |
| Montar a lista ordenada | 22,9 ms (390 KB de ids) |
| Buscar enquanto digita | 2,4 ms (2 044 resultados) |
| Janela visível da lista | 0,1 ms (40 linhas) |
| Criar playlist com 50 000 faixas (hash de tudo, 1ª vez) | 410 ms |
| Mesma operação, hash já calculado | 147 ms |
| Ler a playlist de volta (50 000 itens) | 32,8 ms |
| Playback | 0 underruns · 0,02 s de CPU em 3,3 s |

A capa sintética é pequena; com capa real de 1000×1000 cada álbum novo custa
~5,8 ms de decode e resize, o que somaria ~8 s (em 4 threads) ao *primeiro*
scan de 5 000 álbuns. Rescans não pagam nada disso. O hash é sobre arquivos
sintéticos de ~16 KB; num álbum de verdade o custo desloca de CPU pra I/O —
BLAKE3 satura a leitura bem antes de virar gargalo.

Os números de janela e memória do app (cold start ~530 ms, 0% de CPU parado,
143 MB de RSS) foram medidos na versão `egui`. O app Tauri troca isso pelo
webview do SO — é o custo assumido na decisão lá em cima, e ainda não foi
medido de novo.

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

50k faixas em ~2 s, ocupando 1,2 GB. Como regenerar é barato, o corpus
grande não fica versionado — `testdata/` está no `.gitignore`.

Medir o scan e as consultas contra uma pasta de verdade (rodar duas vezes: a
segunda passada é o que mostra se o caminho incremental está funcionando):

```bash
cargo run --release -p player-core --example scan -- ./testdata/lib50k /tmp/lib.db
cargo run --release -p player-core --example playlist_bench -- ./testdata/lib50k
```

Rodar o player apontado numa pasta:

```bash
cargo tauri dev --config crates/pc-app/tauri.conf.json -- ./testdata/lib50k
```

O sync tem teste ponta a ponta em `crates/sync/tests/sync_e2e.rs` — sobe um
servidor de verdade sobre loopback e baixa uma biblioteca inteira, incluindo
retomada após cancelamento e recusa de device não pareado. Procedimento de
teste manual com celular: [`docs/teste-sync.md`](docs/teste-sync.md).
