# App Android

Compose + ExoPlayer no Kotlin, `player-core` e `yasmine-sync` no Rust,
uniffi entre os dois. O app toca a biblioteca local e baixa a biblioteca do
PC por [sync](sync.md).

`minSdk 26` (Android 8), `targetSdk 35`, ABIs `arm64-v8a`,
`armeabi-v7a`, `x86_64`.

## Onde pegar o APK

Na [página de releases](https://github.com/gabrs-ch/yasmine/releases/latest),
`Yasmine_<versão>_android.apk` — universal, baixa e instala direto. Cada tag
`v*` gera um, no mesmo workflow que empacota o desktop.

O APK é **debug** (assinado com a chave de debug). Instala e roda; o Android
avisa que a origem é desconhecida na primeira vez. Um build release assinado
exigiria um keystore em Secrets — não vale a complexidade enquanto a
distribuição for essa.

Builds intermediários (todo push que toca `android/` ou `crates/`) saem como
artefato do workflow **Android**, em Actions. Precisa de login no GitHub e
expira em 90 dias.

## Compilar

Precisa de JDK 17, Android SDK + NDK 27.1 e `cargo-ndk`. O script instala
tudo **sem root**, dentro de `.toolchain/` (~6 GB livres):

```sh
scripts/build-apk.sh                          # setup se preciso + assembleDebug
scripts/build-apk.sh -PyasmineAbis=arm64-v8a  # um ABI só, build bem mais rápido
```

O APK sai em `android/app/build/outputs/apk/debug/app-debug.apk`.

O Gradle chama `cargo-ndk` pra compilar o `.so` por ABI e roda o
`uniffi-bindgen` pra gerar o Kotlin — tudo em
[`android/app/build.gradle.kts`](../android/app/build.gradle.kts). O bindgen
precisa de um `.so` do **host** pra extrair a metadata (o modo `--library`
não lê binário de outra arquitetura), então o build compila o crate duas
vezes: uma pro host, uma por ABI.

Se a partição do repo estiver apertada, `build-apk.sh` redireciona o cache do
Gradle, o target do Cargo e as pastas de build pra outro disco por symlink —
o build do Android precisa de ~4 GB de rascunho.

## Estrutura

```
app/yasmine/
  YasmineApp.kt          Application: segura o repositório, cria o canal de notificação
  MainActivity.kt        edge-to-edge, pede POST_NOTIFICATIONS, monta o tema
  data/
    LibraryRepository.kt única porta de entrada da FFI — tudo em Dispatchers.IO
  playback/
    PlaybackService.kt   MediaSessionService (ExoPlayer)
    PlayerConnection.kt  liga a UI ao serviço; monta a fila, aplica o ganho
  sync/
    SyncService.kt       foreground service que segura o pull
    SyncBus.kt           StateFlow do progresso + start/cancel
  ui/
    YasmineNav.kt        3 abas + rota de tela cheia do player
    library/ playlists/ pair/ player/ common/ theme/
```

## A fronteira com o Rust

`LibraryRepository` é a única classe que toca a FFI. Toda chamada é
bloqueante do lado Rust, então tudo passa por `withContext(Dispatchers.IO)`.
Um `StateFlow<Int> revision` sobe depois de scan, sync ou qualquer escrita;
as telas observam e recarregam.

Dois objetos vêm da FFI:

- **`YasmineLibrary`** — abre o banco, scan, consultas, playlists, apagar
  faixa. Construído com os caminhos que o Kotlin escolhe (o Rust não
  descobre diretório sozinho no Android).
- **`Syncer`** — identidade estática, ler QR, `pull`/`pullFromAddr`,
  descoberta mDNS, listar e esquecer pareados.

O Rust é o único escritor do SQLite. O Kotlin nunca abre o banco direto.

## Playback

`PlaybackService` é um `MediaSessionService` — notificação, tela de
bloqueio, Bluetooth e Android Auto vêm do media3 sem código nosso.

`PlayerConnection.playTracks` resolve o caminho de cada faixa pela FFI, monta
os `MediaItem` e aplica o ganho do nivelador (`gain_db`) como volume do
ExoPlayer. A capa entra como `artworkUri` apontando pra miniatura de 512px
que o Rust gerou, então a notificação e a tela de bloqueio ganham arte de
graça.

Shuffle e repeat usam `shuffleModeEnabled`/`repeatMode` do próprio
ExoPlayer — o "Shuffle" da Biblioteca liga o modo e começa numa faixa
sorteada.

## Sync em foreground service

O `pull` é longo e não pode morrer quando o usuário sai da tela. Ele roda no
`SyncService`, um foreground service `dataSync` com notificação de
progresso: trocar de aba, minimizar o app ou travar a tela não interrompe.

`SyncBus` é o singleton que liga os dois: o serviço publica
`SyncUiState` (`Idle`/`Running`/`Done`/`Failed`) num `StateFlow`, a tela de
pareamento observa. O composable não segura mais o `pull`, então voltar pra
tela re-observa o progresso em vez de recomeçar. Cancelar manda `cancel()`
pro `Syncer` e para o serviço.

O `dataSync` no Android 15 tem orçamento de ~6 h/dia. Um sync de biblioteca
fica muito abaixo disso.

## Leitura do QR

CameraX + ML Kit, em `QrScanner.kt`. O `PreviewView` usa
`ImplementationMode.COMPATIBLE` de propósito: o modo padrão
(`PERFORMANCE`, `SurfaceView`) desenha numa surface separada que o sistema
compõe **por cima** dos irmãos, e campos de texto do Compose apareciam
atrás/sobre a câmera. `COMPATIBLE` usa `TextureView`, que compõe na
hierarquia de views e respeita clip, scroll e z-order.

## Interface

A paleta e a tipografia são as mesmas do desktop: tokens de
`crates/pc-app/ui/src/styles/app.css` (`--ground #08080A`, `--card`,
`--elevated`, acento `#7C5CFF`) traduzidos pra `darkColorScheme` em
`ui/theme/Theme.kt`, e IBM Plex Sans em `res/font/`. O app é escuro fixo.

Capas aparecem em toda linha, via Coil lendo as miniaturas do cache do Rust.
Sem capa, um gradiente determinístico escolhido por hash — o mesmo truque do
desktop, pra lista nunca ter buraco cinza.

Toque longo abre menu de contexto: numa faixa da Biblioteca, "Add to
playlist" e "Delete from library"; numa faixa dentro de playlist, "Remove
from playlist"; numa playlist, "Rename" e "Delete". Tocar numa faixa toca e
abre o player em tela cheia.

"Delete from library" apaga o arquivo **do celular** e reindexa. Não alcança
o PC, e o próximo sync traz a faixa de volta.

## Ícone

Gerado de `crates/pc-app/assets/icon-source.png` — o mesmo arquivo do
desktop. Adaptativo (`mipmap-anydpi-v26`) com fundo `#08080A` e a arte na
safe zone, mais os PNGs legados por densidade.
