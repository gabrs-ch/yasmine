//! A fila de reprodução.
//!
//! # Duas listas, não uma
//!
//! `tracks` guarda o que foi enfileirado, na ordem natural. `order` é a ordem
//! em que essas faixas vão tocar — a identidade quando o shuffle está
//! desligado, uma permutação quando está ligado. Separar as duas resolve o
//! comportamento que todo player bom tem e que quase nenhum explica:
//!
//! - ligar o shuffle **não** troca a faixa que está tocando, só embaralha o
//!   que vem depois;
//! - desligar volta à ordem natural **a partir de onde você está**, em vez de
//!   pular para outro lugar;
//! - a ordem embaralhada é sorteada uma vez e fica. Sortear a cada `next()`
//!   faria "anterior" mentir e deixaria faixas se repetirem antes de a fila
//!   dar a volta.
//!
//! # Custo
//!
//! `order` é `Vec<u32>` e `tracks` é `Vec<TrackId>`: 12 bytes por faixa, ou
//! 600 KB para uma fila com a biblioteca inteira de 50 000. É por isso que
//! tocar uma faixa da lista pode enfileirar tudo sem pensar duas vezes.

use player_core::TrackId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Repeat {
    #[default]
    Off,
    /// Ao acabar a fila, volta ao começo.
    All,
    /// Repete a faixa atual indefinidamente.
    One,
}

impl Repeat {
    /// Ordem do ciclo do botão: desligado → tudo → uma → desligado.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }
}

#[derive(Debug, Default)]
pub struct Queue {
    tracks: Vec<TrackId>,
    /// Índices em `tracks`, na ordem de reprodução.
    order: Vec<u32>,
    /// Posição dentro de `order`.
    cursor: Option<usize>,
    shuffle: bool,
    repeat: Repeat,
    rng: Rng,
}

impl Queue {
    /// Substitui a fila e posiciona o cursor em `start` (índice na ordem
    /// natural de `tracks`).
    pub fn replace(&mut self, tracks: Vec<TrackId>, start: usize) {
        self.tracks = tracks;
        self.order = (0..u32::try_from(self.tracks.len()).unwrap_or(u32::MAX)).collect();
        self.cursor = (start < self.tracks.len()).then_some(start);

        if self.shuffle {
            // Embaralha o resto mantendo quem está tocando onde está.
            self.reshuffle();
        }
    }

    pub fn clear(&mut self) {
        self.tracks.clear();
        self.order.clear();
        self.cursor = None;
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// Posição na ordem de reprodução, contando de 1 — o que a barra mostra
    /// como "4 / 12". Com shuffle ligado é a posição na ordem embaralhada,
    /// que é a que responde "quanto falta pra fila acabar".
    #[must_use]
    pub fn position(&self) -> Option<usize> {
        self.cursor.map(|cursor| cursor + 1)
    }

    #[must_use]
    pub const fn shuffle(&self) -> bool {
        self.shuffle
    }

    #[must_use]
    pub const fn repeat(&self) -> Repeat {
        self.repeat
    }

    pub const fn set_repeat(&mut self, repeat: Repeat) {
        self.repeat = repeat;
    }

    /// A faixa tocando agora.
    #[must_use]
    pub fn current(&self) -> Option<TrackId> {
        self.at(self.cursor?)
    }

    /// Índice da faixa atual na ordem natural — é o que a lista destaca.
    #[must_use]
    pub fn current_index(&self) -> Option<usize> {
        let cursor = self.cursor?;
        self.order.get(cursor).map(|&i| i as usize)
    }

    /// A próxima faixa, sem mexer no cursor.
    ///
    /// É o que alimenta o gapless: o motor de áudio precisa saber com
    /// antecedência o que abrir.
    #[must_use]
    pub fn peek_next(&self) -> Option<TrackId> {
        self.at(self.next_cursor()?)
    }

    /// Avança e devolve a nova faixa atual.
    pub fn advance(&mut self) -> Option<TrackId> {
        self.cursor = Some(self.next_cursor()?);
        self.current()
    }

    /// Volta uma faixa. Não obedece a `Repeat::One`: quem aperta "anterior"
    /// quer sair da faixa, não ouvi-la de novo.
    pub fn previous(&mut self) -> Option<TrackId> {
        let cursor = self.cursor?;
        let target = if cursor > 0 {
            cursor - 1
        } else if self.repeat == Repeat::All {
            self.order.len().checked_sub(1)?
        } else {
            return None;
        };
        self.cursor = Some(target);
        self.current()
    }

    /// Liga ou desliga o shuffle preservando a faixa atual.
    pub fn set_shuffle(&mut self, shuffle: bool) {
        if self.shuffle == shuffle {
            return;
        }
        self.shuffle = shuffle;

        // Guarda quem está tocando pela posição natural, que é estável nas
        // duas ordens.
        let playing = self.current_index();

        if shuffle {
            self.reshuffle();
        } else {
            self.order = (0..u32::try_from(self.tracks.len()).unwrap_or(u32::MAX)).collect();
        }

        // Recoloca o cursor sobre a mesma faixa na ordem nova.
        self.cursor =
            playing.and_then(|natural| self.order.iter().position(|&i| i as usize == natural));
    }

    /// Embaralha tudo menos a faixa atual, que fica onde está.
    fn reshuffle(&mut self) {
        let fixed = self.cursor;
        let len = self.order.len();
        if len < 2 {
            return;
        }

        // Fisher-Yates de trás pra frente, pulando a posição do cursor.
        for i in (1..len).rev() {
            if Some(i) == fixed {
                continue;
            }
            let mut j = self.rng.below(i + 1);
            if Some(j) == fixed {
                // Trocar com a posição fixa moveria a faixa que está tocando.
                j = if j == 0 { i } else { j - 1 };
            }
            if Some(j) != fixed {
                self.order.swap(i, j);
            }
        }
    }

    fn at(&self, cursor: usize) -> Option<TrackId> {
        let index = *self.order.get(cursor)? as usize;
        self.tracks.get(index).copied()
    }

    /// Para onde o cursor iria, sem mexer nele.
    fn next_cursor(&self) -> Option<usize> {
        let cursor = self.cursor?;
        if self.repeat == Repeat::One {
            return Some(cursor);
        }
        let next = cursor + 1;
        if next < self.order.len() {
            Some(next)
        } else if self.repeat == Repeat::All && !self.order.is_empty() {
            Some(0)
        } else {
            None
        }
    }
}

/// xorshift64*, semeado pelo relógio.
///
/// Um gerador criptográfico aqui seria peso morto: a única exigência é que a
/// ordem não seja previsível a ponto de incomodar o ouvido.
#[derive(Debug)]
struct Rng(u64);

impl Default for Rng {
    fn default() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x2545_F491_4F6C_DD1D, |d| d.as_nanos() as u64);
        Self(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }
}

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) % n as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fila(n: usize, start: usize) -> Queue {
        let mut queue = Queue::default();
        queue.replace((0..n as i64).map(TrackId).collect(), start);
        queue
    }

    #[test]
    fn anda_pra_frente_e_para_no_fim() {
        let mut queue = fila(3, 0);
        assert_eq!(queue.current(), Some(TrackId(0)));
        assert_eq!(queue.advance(), Some(TrackId(1)));
        assert_eq!(queue.advance(), Some(TrackId(2)));
        assert_eq!(queue.advance(), None, "sem repeat, a fila acaba");
    }

    #[test]
    fn repeat_all_da_a_volta_nos_dois_sentidos() {
        let mut queue = fila(3, 2);
        queue.set_repeat(Repeat::All);
        assert_eq!(queue.advance(), Some(TrackId(0)));
        assert_eq!(queue.previous(), Some(TrackId(2)));
    }

    #[test]
    fn repeat_one_fica_na_mesma_faixa() {
        let mut queue = fila(3, 1);
        queue.set_repeat(Repeat::One);
        assert_eq!(queue.peek_next(), Some(TrackId(1)));
        assert_eq!(queue.advance(), Some(TrackId(1)));
    }

    /// "Anterior" tem que tirar você da faixa mesmo em repeat de uma só —
    /// senão o botão não faz nada e parece quebrado.
    #[test]
    fn anterior_escapa_do_repeat_one() {
        let mut queue = fila(3, 1);
        queue.set_repeat(Repeat::One);
        assert_eq!(queue.previous(), Some(TrackId(0)));
    }

    #[test]
    fn peek_next_nao_mexe_no_cursor() {
        let queue = fila(3, 0);
        assert_eq!(queue.peek_next(), Some(TrackId(1)));
        assert_eq!(queue.current(), Some(TrackId(0)));
    }

    /// O comportamento que justifica as duas listas.
    #[test]
    fn ligar_shuffle_nao_troca_a_faixa_tocando() {
        let mut queue = fila(50, 17);
        assert_eq!(queue.current(), Some(TrackId(17)));
        queue.set_shuffle(true);
        assert_eq!(queue.current(), Some(TrackId(17)));
    }

    #[test]
    fn desligar_shuffle_volta_a_ordem_natural_de_onde_esta() {
        let mut queue = fila(50, 17);
        queue.set_shuffle(true);
        queue.advance();
        let atual = queue.current().expect("faixa");

        queue.set_shuffle(false);
        assert_eq!(queue.current(), Some(atual), "pulou de faixa ao desligar");
        // De volta à ordem natural, o próximo é o vizinho de verdade.
        assert_eq!(queue.peek_next(), Some(TrackId(atual.0 + 1)));
    }

    #[test]
    fn shuffle_usa_cada_faixa_exatamente_uma_vez() {
        let mut queue = fila(200, 0);
        queue.set_shuffle(true);

        let mut vistas = vec![queue.current().expect("primeira")];
        while let Some(faixa) = queue.advance() {
            vistas.push(faixa);
        }

        assert_eq!(vistas.len(), 200, "faixa faltando ou repetida");
        vistas.sort_unstable();
        vistas.dedup();
        assert_eq!(vistas.len(), 200, "faixa repetida antes de dar a volta");
    }

    #[test]
    fn shuffle_realmente_embaralha() {
        let mut queue = fila(500, 0);
        queue.set_shuffle(true);

        let ordem: Vec<_> = std::iter::from_fn(|| queue.advance()).collect();
        let natural: Vec<_> = (1..500).map(TrackId).collect();
        assert_ne!(ordem, natural);
    }

    #[test]
    fn fila_vazia_nao_estoura() {
        let mut queue = Queue::default();
        assert_eq!(queue.current(), None);
        assert_eq!(queue.advance(), None);
        assert_eq!(queue.previous(), None);
        queue.set_shuffle(true);
        assert!(queue.is_empty());
    }

    #[test]
    fn ciclo_do_botao_de_repeat() {
        assert_eq!(Repeat::Off.next(), Repeat::All);
        assert_eq!(Repeat::All.next(), Repeat::One);
        assert_eq!(Repeat::One.next(), Repeat::Off);
    }
}
