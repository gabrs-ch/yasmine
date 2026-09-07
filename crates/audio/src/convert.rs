//! Adaptação entre o formato do arquivo e o do dispositivo.
//!
//! **O caminho feliz é este módulo não fazer nada.** O dispositivo é aberto na
//! taxa e no número de canais do arquivo sempre que o hardware aceita, e aí
//! samples passam direto. O que está aqui é o plano B, para quando o
//! dispositivo não oferece a taxa do arquivo.
//!
//! A conversão roda no worker de decodificação, nunca no callback de áudio.

use crate::decode::Spec;

/// Converte blocos intercalados de `from` para `to`.
#[derive(Debug)]
pub struct Converter {
    from: Spec,
    to: Spec,
    resampler: Option<Resampler>,
    /// Buffer intermediário entre o mapeamento de canais e a reamostragem.
    /// Vive aqui para não alocar a cada bloco.
    mapped: Vec<f32>,
}

impl Converter {
    #[must_use]
    pub fn new(from: Spec, to: Spec) -> Self {
        let resampler = (from.sample_rate != to.sample_rate).then(|| {
            Resampler::new(
                f64::from(from.sample_rate) / f64::from(to.sample_rate),
                to.channels as usize,
            )
        });
        Self {
            from,
            to,
            resampler,
            mapped: Vec::new(),
        }
    }

    /// `true` quando arquivo e dispositivo já falam a mesma língua e os
    /// samples podem ir direto pro ring.
    #[must_use]
    pub const fn is_passthrough(&self) -> bool {
        self.from.sample_rate == self.to.sample_rate && self.from.channels == self.to.channels
    }

    /// Converte `input` e **acrescenta** o resultado em `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.is_passthrough() {
            out.extend_from_slice(input);
            return;
        }

        let mapped: &[f32] = if self.from.channels == self.to.channels {
            input
        } else {
            self.mapped.clear();
            map_channels(
                input,
                self.from.channels as usize,
                self.to.channels as usize,
                &mut self.mapped,
            );
            &self.mapped
        };

        match &mut self.resampler {
            Some(resampler) => resampler.process(mapped, out),
            None => out.extend_from_slice(mapped),
        }
    }
}

/// Reencaixa um bloco intercalado em outro número de canais.
///
/// Mono para estéreo duplica, estéreo para mono soma e divide. Acima disso, os
/// canais extras do arquivo são descartados e os que faltam vão a zero —
/// downmix 5.1 de verdade fica para quando alguém pedir.
fn map_channels(input: &[f32], from: usize, to: usize, out: &mut Vec<f32>) {
    if from == 0 || to == 0 {
        return;
    }
    let frames = input.len() / from;
    out.reserve(frames * to);

    for frame in input.chunks_exact(from) {
        match (from, to) {
            (1, _) => {
                for _ in 0..to {
                    out.push(frame[0]);
                }
            }
            (_, 1) => {
                let sum: f32 = frame.iter().take(2).sum();
                out.push(sum / frame.len().min(2) as f32);
            }
            _ => {
                for ch in 0..to {
                    out.push(frame.get(ch).copied().unwrap_or(0.0));
                }
            }
        }
    }
}

/// Reamostrador linear.
///
/// Interpolação linear é barata e tem aliasing audível em conversões
/// agressivas. É aceitável porque este é o caminho de exceção: o normal é o
/// dispositivo abrir na taxa do arquivo e nem passar por aqui.
#[derive(Debug)]
struct Resampler {
    /// Quantos frames de entrada por frame de saída.
    ratio: f64,
    /// Posição fracionária dentro do fluxo virtual `[último frame] ++ entrada`.
    pos: f64,
    /// Último frame do bloco anterior, para interpolar através da emenda.
    last: Vec<f32>,
    channels: usize,
}

impl Resampler {
    fn new(ratio: f64, channels: usize) -> Self {
        Self {
            ratio,
            pos: 0.0,
            last: vec![0.0; channels],
            channels,
        }
    }

    fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.channels == 0 || input.is_empty() {
            return;
        }
        let frames = input.len() / self.channels;
        if frames == 0 {
            return;
        }

        // Fluxo virtual: o frame 0 é o último do bloco anterior, o resto é a
        // entrada. É o que faz a emenda entre blocos não estalar.
        let total = frames + 1;
        let frame = |index: usize, ch: usize| -> f32 {
            if index == 0 {
                self.last[ch]
            } else {
                input[(index - 1) * self.channels + ch]
            }
        };

        while self.pos + 1.0 < total as f64 {
            let index = self.pos as usize;
            let frac = (self.pos - index as f64) as f32;
            for ch in 0..self.channels {
                let a = frame(index, ch);
                let b = frame(index + 1, ch);
                out.push(a + (b - a) * frac);
            }
            self.pos += self.ratio;
        }

        // Guarda o último frame real da entrada para interpolar contra o
        // primeiro do próximo bloco.
        let tail = (frames - 1) * self.channels;
        self.last
            .copy_from_slice(&input[tail..tail + self.channels]);
        self.pos -= (total - 1) as f64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn spec(sample_rate: u32, channels: u16) -> Spec {
        Spec {
            sample_rate,
            channels,
        }
    }

    #[test]
    fn mesmo_formato_passa_direto() {
        let mut conv = Converter::new(spec(44100, 2), spec(44100, 2));
        assert!(conv.is_passthrough());

        let mut out = Vec::new();
        conv.process(&[0.1, 0.2, 0.3, 0.4], &mut out);
        assert_eq!(out, vec![0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn mono_vira_estereo_duplicando() {
        let mut out = Vec::new();
        map_channels(&[0.5, -0.5], 1, 2, &mut out);
        assert_eq!(out, vec![0.5, 0.5, -0.5, -0.5]);
    }

    #[test]
    fn estereo_vira_mono_pela_media() {
        let mut out = Vec::new();
        map_channels(&[1.0, 0.0, 0.5, 0.5], 2, 1, &mut out);
        assert_eq!(out, vec![0.5, 0.5]);
    }

    /// Dobrar a taxa tem que dobrar a contagem de frames, não multiplicá-la
    /// por acaso.
    #[test]
    fn reamostrar_para_o_dobro_dobra_os_frames() {
        let mut conv = Converter::new(spec(22050, 1), spec(44100, 1));
        let entrada: Vec<f32> = (0..100).map(|n| n as f32).collect();

        let mut out = Vec::new();
        conv.process(&entrada, &mut out);

        let esperado = entrada.len() * 2;
        assert!(
            out.len().abs_diff(esperado) <= 2,
            "esperava ~{esperado} samples, saiu {}",
            out.len()
        );
    }

    /// A emenda entre blocos é onde reamostrador ruim estala: processar em
    /// dois pedaços tem que dar quase o mesmo que processar de uma vez.
    #[test]
    fn emenda_entre_blocos_nao_perde_continuidade() {
        let entrada: Vec<f32> = (0..200).map(|n| n as f32).collect();

        let mut inteiro = Vec::new();
        Converter::new(spec(44100, 1), spec(48000, 1)).process(&entrada, &mut inteiro);

        let mut partido = Vec::new();
        let mut conv = Converter::new(spec(44100, 1), spec(48000, 1));
        conv.process(&entrada[..100], &mut partido);
        conv.process(&entrada[100..], &mut partido);

        assert!(inteiro.len().abs_diff(partido.len()) <= 1);
        // A rampa é linear, então a interpolação tem que reproduzi-la de perto
        // dos dois jeitos.
        for (a, b) in inteiro.iter().zip(&partido).take(150) {
            assert!((a - b).abs() < 0.5, "descontinuidade: {a} vs {b}");
        }
    }

    #[test]
    fn reduzir_a_taxa_reduz_os_frames() {
        let mut conv = Converter::new(spec(48000, 2), spec(24000, 2));
        let entrada: Vec<f32> = (0..400).map(|n| n as f32).collect();

        let mut out = Vec::new();
        conv.process(&entrada, &mut out);

        assert!(out.len().abs_diff(entrada.len() / 2) <= 4);
    }
}
