//! PRNG determinístico (xorshift64*).
//!
//! Determinismo é o ponto: a mesma seed dá a mesma biblioteca, byte a byte.
//! Sem isso, comparar o tempo de scan entre dois commits não significa nada,
//! porque o corpus mudou junto.

pub struct Rng(u64);

impl Rng {
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        // Seed 0 trava o xorshift em zero para sempre.
        Self(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    pub const fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Inteiro em `[0, n)`. Módulo enviesado, e tudo bem: é dado de teste.
    pub const fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// Inteiro em `[lo, hi]`.
    pub const fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + self.below(hi - lo + 1)
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}
