# Protocolo de sync (v1)

Spec do que `crates/sync` fala no fio. O recurso como um todo — o que o
usuário faz, modelo de confiança, merge, garantias — está em
[`sync.md`](sync.md); aqui é só o formato. Implementado em
[`crates/sync/src/protocol.rs`](../crates/sync/src/protocol.rs) (mensagens),
[`channel.rs`](../crates/sync/src/channel.rs) (canal), `server.rs`, `client.rs`.

## Camadas

```
  aplicação:  Msg  (postcard)                 protocol.rs
  transporte: Noise_IK_25519_ChaChaPoly_BLAKE2s   channel.rs
  rede:       TCP
  descoberta: mDNS  _yasmine-sync._tcp.local.  discovery.rs   (opcional)
```

Prazos: handshake 10 s dos dois lados; depois 120 s por leitura no host e 60 s
no celular (180 s só na resposta ao primeiro `Hello`, que espera o humano do PC
confirmar o pareamento). O host serve no máximo 4 conexões simultâneas. Sem
isso, quem estiver na LAN abre conexões mudas e segura uma thread em cada uma.

## 1. Pareamento (QR)

O host mostra um QR com:

```
yasmine://pair?k=<base64url da chave pública estática, 32 bytes>
              &h=<ip da LAN>        (opcional)
              &p=<porta TCP>        (opcional)
              &n=<nome legível>     (opcional)
```

`k` **é** o `DeviceId` do host. `h`/`p` dispensam o mDNS no primeiro contato
(rede que bloqueia multicast). Sem eles, o celular resolve o host pelo `k` via
mDNS.

## 2. Handshake — `Noise_IK`

`IK`: o iniciador (celular) já conhece a estática do respondedor (veio no
QR). Mensagem 1 leva a estática do celular **cifrada**; o host descobre quem
é o par ao processá-la e decide ali se aceita (device na tabela `device`, ou
confirmação do usuário). Rejeição = `Msg::Error` e a conexão cai.

- Records de handshake: prefixo `u16` big-endian + bytes.
- Se a estática do host não bate com o `k` do QR, a msg 2 não decifra —
  MITM detectado pelo próprio handshake, sem PKI.

## 3. Mensagens (`Msg`, pós-handshake)

Serialização `postcard`. Cada `Msg` lógico é `u32(len) ++ postcard(msg)`,
fatiado em records Noise de ≤ 65519 B de plaintext; cada record vai ao socket
com prefixo `u16` do tamanho cifrado. `MAX_FRAME` = 64 MiB (teto anti-DoS).

| # | Mensagem | Sentido | Conteúdo |
|---|----------|---------|----------|
| 1 | `Hello { proto, device_name }` | ambos | versão do protocolo, nome |
| 2 | `User(UserLayer)` | host→ | snapshot inteiro da camada do usuário |
| 3 | `Have { hashes: [[u8;32]] }` | celular→ | content_hash que o celular já tem, ordenado |
| 4 | `Tracks([TrackMeta])` | host→ | metadata das faixas que o celular **não** tem |
| 5 | `NeedBlob { hash, from }` | celular→ | pede o áudio de um hash a partir do byte `from` |
| 6 | `Blob { hash, offset, data, last }` | host→ | pedaço de 128 KiB; `last` no final |
| 7 | `Done` | celular→ | fim da sessão |
| — | `Error { msg }` | ambos | aborta |

O `proto` do `Hello` é **conferido** dos dois lados: versão diferente vira
`Error` limpo em vez de erro de parse mais adiante.

Ordem numa sessão: `Hello`×2 → `User` → `Have` → `Tracks` → (`NeedBlob` →
`Blob…`)×N → `Done`. A camada do usuário vem **antes** do áudio: a biblioteca
do celular já aparece povoada enquanto os arquivos chegam.

### `TrackMeta`

`{ hash, ext, size, title?, artist?, album?, album_artist?, disc_no?,
track_no?, year?, genre? }`. Propriedades de stream (sample rate etc.) e capa
**não** viajam: saem do próprio arquivo quando o celular roda `scan` na pasta
baixada. A capa é regenerada das tags pelo `ArtCache` — transferir imagem
seria banda gasta pra reproduzir o que o receptor deriva de graça.

Todo campo aqui é **entrada não confiável**: o pareamento autentica *quem* está
do outro lado, não garante que o que ele diz seja honesto. `artist`, `album` e
`title` passam por `sanitize`, e `ext` por `sanitize_ext` (token alfanumérico de
até 8 caracteres, senão `bin`) — sem isso um `ext` com `../` escapa da pasta da
biblioteca, já que ele é colado cru no nome do arquivo.

### `UserLayer`

Linhas cruas de `device`, `playlist`, `playlist_item`, `play_count`,
`track_state`. O merge ([`merge.rs`](../crates/sync/src/merge.rs)) decide o
que fica — as regras e o porquê de cada uma estão em
[`sync.md`](sync.md#merge-da-camada-do-usuário).

## 4. Download

Por faixa faltante: `NeedBlob { hash, from }`. O celular grava em
`dest/.incoming/<hex>.part` (append). `from` = tamanho do `.part` que já
existe → **retomável** de graça: cancelar deixa os `.part`, a próxima chamada
continua. Ao receber o `last`, verifica o BLAKE3 do arquivo inteiro contra o
`hash` pedido; se não bater, descarta e conta em `hash_mismatch`. Batendo,
move pra `dest/<artista>/<álbum>/<NN título>.<ext>` (higienizado; sem tags,
cai pra `dest/<hex>.<ext>`).

Quem decide que o arquivo acabou é o `last` do host, então o celular corta a
conexão se chegar mais byte do que o `size` anunciado — senão um host hostil
escreve até encher o armazenamento. O destino também é conferido contra o
`dest` antes de qualquer escrita.

No fim: `player_core::scan(dest)` indexa tudo (tags, stream, capa) e
`ensure_hashes` nas faixas novas — os itens de playlist, que apontam por
`track_key`, viram faixas tocáveis.

## Otimizações que ficaram pra depois

A v1 é o caminho simples de propósito; cada item abaixo troca código por
banda ou tempo, e nenhum é gargalo na escala atual.

- **Resumo de hashes** (Bloom/Merkle) em vez de mandar N×32 bytes no `Have`.
- **Diff por `(size, mtime)`** antes de forçar `ensure_hashes` no host — hoje
  o host hasheia a biblioteca inteira na primeira conexão.
- **Sync incremental** usando `device.last_sync_at`.
- Escopo seletivo do download (hoje é sempre "biblioteca inteira").
