# Testar o sync ponta a ponta

PC e celular na **mesma rede Wi-Fi**. O desktop precisa estar em schema v5
(release `v0.5.6` ou posterior). Descrição do recurso em [`sync.md`](sync.md).

## 1. PC — subir o servidor

Pelo app: abre o Yasmine, clica no ícone de telefone na barra do topo. O
painel mostra o QR e o status. (Precisa de uma pasta de música já apontada —
sem biblioteca não há o que servir.)

Sem abrir a UI:

```sh
cargo run -p yasmine-sync-host -- --mdns              # QR no terminal
cargo run -p yasmine-sync-host --features gui -- --gui --mdns   # janela
```

A variante `--gui` precisa de `libxkbcommon-dev` e `libgl1-mesa-dev`. Se a
biblioteca ainda não foi indexada nesse banco: `--music ~/Musica`.

**No primeiro teste, use um dos dois, não os dois juntos.** Os dois abrem o
mesmo `library.db`; o WAL aguenta, mas comece simples.

## 2. Celular

Instala o `Yasmine_<versão>_android.apk`
([releases](https://github.com/gabrs-ch/yasmine/releases/latest)). Abre →
aba **Sync** → permite a câmera → aponta pro QR → **Download**.

O progresso passa por *connecting → merging playlists → comparing libraries
→ downloading → indexing*. Enquanto baixa, saia da tela de propósito: a
notificação tem que continuar contando. Trave o celular também. Voltar pro
app tem que mostrar o progresso de onde está, não recomeçar.

**Cancelar e retomar**: cancela no meio, confirma que os `.part` ficaram, e
sincroniza de novo — tem que continuar de onde parou, não do zero.

No fim, na **Library**: faixas tocando pelo ExoPlayer (com notificação e
tela de bloqueio), capas visíveis, e as **Playlists** povoadas.

## 3. Merge — segunda rodada

O que valida a camada do usuário:

1. No celular: dá um rating numa faixa, cria uma playlist, apaga um item de
   outra.
2. Sincroniza de novo com o mesmo PC.
3. Confere:
   - o item apagado **não** voltou (túmulo, schema v5);
   - rating e posição de retomada não se sobrescreveram (LWW por campo);
   - contagem de plays somou em vez de substituir (G-Counter).

## 4. PC depois

Reabre o app: o `library.db` continua legível, playlists intactas. Apagar um
item de playlist na UI faz ele sumir da lista, e
`SELECT count(*) FROM playlist_item WHERE deleted = 1` sobe.

Confere também que o sync **não** tocou nos arquivos do PC — nem mtime, nem
pasta, nem nada. O servidor só lê.

## Arestas conhecidas

- O app desktop cacheia o estado em memória. Pra ver o que chegou pelo sync,
  reabra (ou troque de fonte e volte).
- Se o desktop estiver aberto escrevendo durante o sync, pode dar
  `SQLITE_BUSY` — `busy_timeout = 5000` costuma segurar; se der erro, feche
  o desktop e repita.
- A primeira conexão hasheia a biblioteca inteira no PC. Numa biblioteca
  grande isso demora antes de o download começar. Só na primeira.
- Rede sem mDNS: "Enter address manually" na tela de pareamento aceita o
  device id em hex (vem no QR) e `ip:porta`.
