# Sync local: PC → Android

O Yasmine sincroniza biblioteca entre um PC e um celular na mesma rede, sem
conta, sem nuvem e sem servidor intermediário. O PC mostra um QR code, o
celular lê, e a biblioteca inteira do PC — arquivos de áudio, playlists,
contagem de plays, rating — desce pro celular. Depois disso o celular tem
uma biblioteca própria: dá pra desligar o Wi-Fi e continuar ouvindo.

Este documento descreve o recurso como sistema. O formato exato das
mensagens no fio está em [`protocolo-sync.md`](protocolo-sync.md).

## O que o usuário faz

No PC, duas portas de entrada pro mesmo servidor:

- **App desktop** — botão do telefone na barra do topo. Abre um painel com
  o QR e o status ao vivo ("aguardando", "fulano conectou", "enviando 45%").
  Fechar o painel derruba o servidor.
- **`yasmine-sync-host`** — binário à parte, pra quem não quer abrir a UI.
  `cargo run -p yasmine-sync-host -- --mdns` imprime o QR no terminal em
  blocos unicode; `--features gui --gui` abre uma janela com o QR grande.

No celular: aba **Sync** → aponta a câmera no QR → confirma o nome do PC →
**Download**. O progresso roda num *foreground service*, então dá pra sair
da tela, minimizar o app ou travar o celular que o download continua; a
notificação mostra a porcentagem. Cancelar interrompe e o que já baixou
fica — a próxima tentativa retoma de onde parou.

Rede que bloqueia mDNS e QR que não lê: a tela de pareamento tem
"Enter address manually", que aceita o device id em hex e `ip:porta`.

## Topologia

```
        PC (servidor)                         celular (cliente)
   ┌───────────────────────┐            ┌────────────────────────┐
   │ library.db            │            │ library.db             │
   │ pasta de música       │            │ Musica/                │
   │                       │            │                        │
   │ Server::bind(0.0.0.0) │◀───TCP─────│ pull()                 │
   │   só LEITURA do áudio │   Noise_IK │   escreve tudo          │
   └───────────────────────┘            └────────────────────────┘
```

O sync é **pull**: quem puxa é o celular, sempre. O PC responde a pedidos e
não inicia nada. Isso não é detalhe de implementação, é a garantia central
do recurso — ver [Garantias](#garantias-e-limites).

## Confiança: o QR é a credencial

Não existe conta, senha nem PKI. O QR carrega a **chave pública estática
Noise do PC**, que *é* o `DeviceId` dele — parear e identificar são a mesma
operação:

```
yasmine://pair?k=<chave pública, base64url>&h=<ip>&p=<porta>&n=<nome>
```

Quem escaneou o QR está autorizado. O handshake usa **Noise_IK**: o celular
já conhece a estática do PC (veio no QR) e manda a própria cifrada na
primeira mensagem. Consequências práticas:

- **MITM cai sozinho.** Se a chave de quem atendeu não for a que o QR
  prometeu, a segunda mensagem do handshake não decifra. Não há certificado
  pra validar nem autoridade pra confiar; a comparação é direta.
- **O canal é cifrado e autenticado dos dois lados** depois do handshake.
  Ninguém na LAN lê o que passa.
- **A janela de exposição é curta.** O servidor só existe enquanto o painel
  está aberto (ou o `sync-host` rodando). Fechou, a porta some.
- **O que o QR concede é leitura da biblioteca.** Um device pareado pode
  baixar; não pode apagar, mover nem escrever nada no PC.

O par conhecido fica gravado na tabela `device` dos dois lados, com
`paired_at`. O app Android lista os pareados e tem "Forget" pra esquecer um.

A chave secreta mora em `meta['sync_sk']`, dentro do próprio `library.db`.
Mesmo nível de confiança do resto do arquivo, que já guarda a biblioteca
inteira — proteger a chave a mais que o índice não protegeria nada.

## As duas camadas

O que viaja se divide em duas, com regras de propriedade diferentes. É a
decisão que barateia o sync inteiro.

**Camada derivada — áudio, endereçada por conteúdo.** A identidade de uma
faixa é o BLAKE3 do arquivo (`track.content_hash`), não o `track.id`, que é
autoincrement local e difere em cada máquina. O protocolo então é "tenho /
não tenho este hash": o celular manda os hashes que já tem, o PC responde a
metadata do que falta, e os bytes descem por hash. **Conflito não existe
nessa camada** — dois arquivos com o mesmo hash são o mesmo arquivo.

**Camada do usuário — merge de verdade.** Playlists, `play_count`,
`track_state` (rating, posição de retomada) só existem no banco; não dá pra
reconstruir a partir dos arquivos. Essa é a única parte que precisa de
resolução de conflito, e ela é pequena — viaja inteira, num snapshot só, e
chega **antes** do áudio. A biblioteca do celular aparece povoada enquanto
os arquivos ainda estão descendo.

Duas coisas deliberadamente **não** viajam:

- **Capa de álbum.** É regenerada no celular a partir das tags dos arquivos
  baixados, pelo mesmo `ArtCache` do scan normal. Transferir imagem custaria
  banda pra reproduzir algo que o celular consegue derivar de graça.
- **Propriedades de stream** (sample rate, canais, duração). Saem do próprio
  arquivo quando o celular indexa.

## A sessão, fase a fase

`Phase` em [`client.rs`](../crates/sync/src/client.rs) — é o que a barra de
progresso mostra.

| Fase | O que acontece |
|---|---|
| `Connecting` | TCP + handshake Noise_IK. A verificação da chave do PC contra a do QR é o próprio handshake. |
| `MergingUserData` | O PC manda `UserLayer` (snapshot cru de `device`, `playlist`, `playlist_item`, `play_count`, `track_state`). `merge::apply` resolve e grava. |
| `FetchingList` | O celular manda `Have` com os hashes que tem; o PC responde `Tracks` com a metadata do que falta. |
| `Downloading` | Um `NeedBlob`/`Blob…` por faixa faltante, em pedaços de 128 KiB. |
| `Indexing` | `player_core::scan` na pasta baixada + `ensure_hashes` nas faixas novas. |
| `Done` | Fim. Os itens de playlist, que apontam por `track_key`, viram faixas tocáveis. |

Para responder "o que eu tenho que você não tem", o PC precisa dos hashes da
própria biblioteca — então a **primeira conexão hasheia a biblioteca
inteira** (`ensure_hashes` em `server.rs`). É o custo inerente de comparar
por conteúdo. O hash fica gravado; conexões seguintes não pagam de novo.

## Download retomável

Cada faixa desce pra `dest/.incoming/<hex>.part`, em append. O campo `from`
do `NeedBlob` é o tamanho do `.part` que já existe — retomar sai de graça,
sem estado extra: cancelar deixa os `.part` no lugar e a chamada seguinte
continua do byte onde parou.

Quando o último pedaço chega, o BLAKE3 do arquivo inteiro é conferido contra
o hash que foi pedido. Batendo, o arquivo vira
`dest/<artista>/<álbum>/<NN título>.<ext>` (nome higienizado; sem tags, cai
pra `dest/<hex>.<ext>`). Não batendo, é descartado e contado em
`hash_mismatch` no relatório final. Arquivo corrompido no meio do caminho
nunca entra na biblioteca.

## Merge da camada do usuário

Todas as regras convergem: aplicar o mesmo snapshot duas vezes dá o mesmo
resultado. Testado em `merge::tests` e no `sync_e2e`.

| Tabela | Regra | Por quê |
|---|---|---|
| `playlist` | LWW por `updated_at`; empate desempata pelo maior `origin` (bytes). Respeita `deleted`. | Renomear/apagar é uma edição só; a última ganha. O desempate por `origin` torna a operação determinística mesmo com relógios idênticos. |
| `playlist_item` | União por `(playlist_id, position)`. `deleted`/`deleted_at` fazem LWW. | `position` é índice fracionário, então dois devices reordenando ao mesmo tempo não colidem. A união preserva o que cada lado acrescentou. |
| `play_count` | G-Counter: `MAX(count)` por `(track_key, device_id)`. Total = `SUM`. | LWW aqui estaria **errado**: se o PC tocou 3× e o celular 2×, LWW guardaria 3 e perderia 2. Contando por device e somando, nenhum play se perde. |
| `track_state` | LWW **por campo**: `rating` pelo maior `rating_updated_at`, `resume_pos_ms` pelo maior `resume_updated_at`, `last_played_at` pelo `MAX`. | Dá pra mudar o rating num device e a posição de retomada no outro sem um sobrescrever o outro. |

**Apagar precisa de túmulo.** Sem uma marca de "apaguei", "não tenho essa
linha" é indistinguível de "nunca tive", e a união reintroduz o que foi
removido no próximo encontro. Por isso `playlist.deleted` existe desde o
começo, e `playlist_item` ganhou `deleted`/`deleted_at` no **schema v5**:
`playlist::remove` marca em vez de apagar a linha, e `items`/`all`/
`cover_hashes` filtram `deleted = 0`.

## Descoberta

O PC publica `_yasmine-sync._tcp.local.` por mDNS, com TXT `id=<hex da
chave>` e `name=<nome legível>`. O celular casa pelo `id` que já tem do
pareamento.

A porta é **0** — o SO escolhe uma livre — e vai publicada no QR (`p=`) e no
TXT do mDNS. Sem porta fixa não há conflito com nada. `--port N` força uma,
se o firewall exigir.

Três caminhos, em ordem de preferência: `h`/`p` do QR → mDNS → IP digitado à
mão. O mDNS é melhor-esforço: se falhar ao subir, o QR ainda carrega
endereço e porta e o pareamento funciona igual.

## Garantias e limites

**O PC nunca é modificado pelo celular.** O `Server`
([`server.rs`](../crates/sync/src/server.rs)) só lê os arquivos de áudio. A
única escrita que ele faz no índice é preencher `track.content_hash` dos
próprios arquivos — cache derivado, e o PC é dono deles. Não existe caminho
de código onde o celular mande o PC apagar, mover ou alterar coisa alguma.
Apagar uma faixa no celular, ou desinstalar o app, não alcança o PC.

**O pareamento autentica quem fala, não o que ele fala.** O handshake prova
que do outro lado está a chave do QR — não que ela seja honesta. Por isso o
celular trata todo campo que vem do PC como entrada não confiável: a extensão
e as tags são higienizadas antes de virarem caminho (senão um `ext` com `../`
escreve fora da pasta da biblioteca), o destino é conferido contra a pasta, e
a faixa é cortada se chegar mais byte do que o `size` anunciado. Vale tanto
pro PC comprometido quanto pro que só tem um índice corrompido.

**A v1 é unidirecional.** O celular faz merge do que veio do PC, mas não
manda nada de volta. Uma playlist criada no celular fica no celular. O
`UserLayer` e o `merge` já são simétricos — o que falta é o sentido inverso
da sessão, não o algoritmo.

**Comparação sempre completa.** `device.last_sync_at` é gravado, mas a v1
compara as bibliotecas inteiras em toda conexão. `Have` manda a lista crua
de hashes: 50 000 faixas = 1,6 MB numa mensagem. Cabe, e é simples.

**Escopo do download é a biblioteca inteira.** Não há seleção de artista,
álbum ou playlist.

**Uma conexão por vez basta** pro caso de uso, mas cada conexão roda na
própria thread — dois celulares ao mesmo tempo funcionam. O teto é 4
simultâneas, e toda conexão tem prazo (10 s pro handshake, 120 s por leitura
depois): sem isso alguém na LAN abre conexões mudas e segura uma thread do PC
em cada uma.

**Metadata editada dos dois lados** gera hashes diferentes, e as duas
versões coexistem como faixas distintas. O sync reconcilia por conteúdo, não
por "nome parecido".

## Onde os dados moram

| | PC | Android |
|---|---|---|
| Índice | `<data_dir>/library.db` (`ProjectDirs("Yasmine")`) | `filesDir/library.db` (privado do app) |
| Áudio | a pasta que o usuário apontou | `getExternalFilesDir("Musica")` |
| Cache de capa | `<cache_dir>/art/` | `cacheDir/yasmine/art/` |
| Chave estática | `meta['sync_sk']` no `library.db` | idem |

O app desktop e o `yasmine-sync-host` abrem o **mesmo** `library.db` —
ambos resolvem por `ProjectDirs::from("", "", "Yasmine")`. Por isso os dois
lados têm que concordar no schema; foi o que motivou linearizar a migração
do túmulo como v5.

No Android, a pasta de música é armazenamento **privado do app**: o Android
a apaga junto ao desinstalar. É coerente com o modelo de espelho — para ter
de volta, sincroniza de novo. Se um dia fizer sentido que as músicas
sobrevivam à desinstalação, o caminho é `MediaStore`/`/sdcard/Music`, com a
permissão e o escopo de storage que isso implica.

## Decisões

**`Noise_IK`, não `KK`.** `KK` exige que os dois lados já conheçam a estática
um do outro — o que não é verdade no primeiro pareamento. Com `IK`, o
primeiro contato e a reconexão usam o mesmo pattern: o celular conhece a
estática do PC pelo QR e apresenta a própria cifrada. Um pattern só, menos
código e menos caminho pra testar.

**`snow` com o resolver RustCrypto puro, sem `ring`.** O `ring` precisa de
toolchain C pra cross-compilar; o resolver em Rust puro atravessa pro NDK
sem nada disso.

**Lista crua de hashes no `Have`, não filtro de Bloom nem árvore de Merkle.**
O resumo economiza banda numa mensagem que já cabe. Bloom traz falso-positivo
(faixa que não desce), Merkle traz código. A otimização está documentada
porque vai fazer sentido em biblioteca muito maior, não porque faça agora.

**RMS e capa não viajam; o celular deriva.** Mesma lógica: transferir o que o
receptor consegue recalcular é banda gasta à toa.

**`content_hash` é preguiçoso.** Nasce `NULL` e só é preenchido quando
alguém precisa — faixa entrando em playlist, ou o sync comparando
bibliotecas. Hashear tudo no scan multiplicaria o custo da primeira varredura
sem ninguém ter pedido sync.

## O que ficou pra depois

- Sentido inverso (celular → PC).
- Diff por `(size, mtime)` antes de forçar `ensure_hashes` no PC.
- Sync incremental de verdade, usando `device.last_sync_at`.
- Resumo de hashes (Bloom/Merkle) no lugar da lista crua.
- Escopo seletivo do download.
