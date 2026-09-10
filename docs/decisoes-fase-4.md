# Fase 4 — decisões

Fecha os "pontos em aberto" do handoff do Yasmine
(`docs/contexto-android.md` lá), com o que este repo implementou.

## Túmulo de `playlist_item` — **resolvido, schema v4**

`vendor/player-core` sobe pra `SCHEMA_VERSION = 4`: `playlist_item` ganha
`deleted INTEGER NOT NULL DEFAULT 0` e `deleted_at INTEGER NOT NULL DEFAULT 0`
(migração incremental em `db.rs`, `schema.sql` intocado — mesma regra das
v2/v3). `playlist::remove` marca `deleted = 1, deleted_at = now` em vez de
`DELETE`; `items`/`all` filtram `deleted = 0`. Sem isso, um item apagado num
device volta no próximo sync.

## Padrão Noise — **`Noise_IK_25519_ChaChaPoly_BLAKE2s`**

`IK`, não `KK`: o primeiro pareamento e a reconexão usam o mesmo pattern. O
celular conhece a estática do host pelo QR e manda a própria cifrada na msg 1;
o host autentica de volta e decide se aceita (a estática do celular pode
ainda não estar na tabela `device` — aí o host pergunta ao usuário).
`snow` com o resolver RustCrypto puro (sem `ring`), pra cross-compilar pro
NDK sem toolchain C.

## Service name mDNS — **`_yasmine-sync._tcp.local.`**

TXT: `id=<hex da chave pública>`, `name=<nome legível>`. O celular casa pelo
`id` que já tem do pareamento.

## Porta — **`0` (o SO escolhe)**, publicada no QR e no mDNS

Sem porta fixa pra brigar. O QR carrega `p=<porta>`; o mDNS também. `--port N`
no host força uma, se precisar de firewall.

## QR carrega host:porta? — **sim, opcional**

`k` (chave) sempre; `h`/`p` quando dá — dispensa mDNS no primeiro contato.
Sem `h`/`p`, resolve por mDNS. Campo de IP manual no app cobre rede sem
nenhum dos dois.

## Resumo de hashes — **lista crua na v1**

`Have { hashes: Vec<[u8;32]> }` ordenada. Bloom/Merkle é otimização
documentada, não v1. Uma biblioteca de 50k faixas = 1,6 MB de hashes numa
mensagem — cabe.

## Escopo do download — **biblioteca inteira**

Decisão de UX confirmada. Barra de progresso + cancelar. Retomável.

## Capa — **regenerada das tags no celular**

Não viaja no fio. Depois do download, `scan` roda na pasta e o `ArtCache`
gera as miniaturas das tags dos próprios arquivos — o mesmo caminho do scan
normal, quase de graça.

## Faixa com hash divergente (metadata editada nos dois lados)

Coexistem como faixas distintas (o sync reconcilia por conteúdo, não por
"nome parecido"). Aceito como no doc original — sem reconciliação especial
na v1.

## Sync incremental vs full — **full na v1**

`device.last_sync_at` é gravado (pelo merge), mas a v1 sempre compara as
bibliotecas inteiras. O diff por `(size, mtime)` e o "desde a última vez"
ficam pra uma v2, junto com o resumo de hashes.

## Merge da camada do usuário

| Tabela | Regra |
|---|---|
| `playlist` | LWW por `updated_at`; empate → maior `origin` (bytes). Respeita `deleted`. |
| `playlist_item` | união por `(playlist_id, position)`; `deleted`/`deleted_at` fazem LWW (maior `deleted_at` ganha). |
| `play_count` | G-Counter: `MAX(count)` por `(track_key, device_id)`. Total = `SUM`. |
| `track_state` | LWW **por campo**: `rating` pelo maior `rating_updated_at`, `resume_pos_ms` pelo maior `resume_updated_at`, `last_played_at` pelo `MAX`. |

Todos convergem: aplicar o mesmo snapshot duas vezes dá o mesmo resultado
(testado em `merge::tests` e no `sync_e2e`).
