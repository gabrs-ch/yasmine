//! Pareamento, descoberta na LAN e sync entre devices. **Fase 4.**
//!
//! Crate isolado de propósito: é o único que fala com a rede e o único que
//! mexe com cripto, então merece revisão mais cuidadosa que o resto.
//!
//! Desenho decidido, para quando a fase chegar:
//!
//! - **Pareamento** por QR code, trocando a chave pública estática do Noise.
//!   A chave *é* o `DeviceId` — parear e identificar são a mesma operação.
//! - **Descoberta** por mDNS na LAN. IP digitado à mão é o plano B.
//! - **Canal** Noise (`snow`), autenticado pelas chaves trocadas no
//!   pareamento.
//! - **Duas camadas, dois algoritmos.** Áudio é endereçado por conteúdo
//!   (BLAKE3): o merge é "tenho/não tenho este hash", conflito não existe.
//!   Estado do usuário faz merge de verdade — LWW por campo para rating e
//!   posição, `MAX` por (faixa, device) para contagem de plays.
//! - **Hash só no caminho lento.** Compara `(tamanho, mtime)` primeiro e só
//!   calcula BLAKE3 quando diverge; hashear a biblioteca inteira a cada sync
//!   seriam minutos de I/O por nada.
