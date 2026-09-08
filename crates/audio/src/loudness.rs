//! Nivelador de volume: mede o quão alto uma faixa soa e deriva um ganho que
//! a deixa perto do resto da biblioteca.
//!
//! # Por que RMS e não EBU R128/ReplayGain de verdade
//!
//! A medida "correta" de volume percebido usa filtro de ponderação-K e
//! gating de trechos silenciosos (ITU-R BS.1770). Implementar esse filtro do
//! zero é boa parte do trabalho de uma biblioteca de áudio inteira, para um
//! ganho de precisão que não muda a decisão prática: RMS do sinal decodificado
//! já resolve "essa faixa é gravada mais baixo que as outras" na esmagadora
//! maioria dos casos, com uma fração do código e sem risco de um filtro mal
//! implementado produzir número errado silenciosamente.
//!
//! # Por que o pico entra na conta
//!
//! Sem ele, uma faixa gravada baixo mas com transientes agudos (uma pancada
//! de bateria, um talo cortado) receberia o ganho cheio calculado pelo RMS e
//! estouraria 0 dBFS nesses trechos — a faixa ficaria mais alta, mas
//! distorcida. O ganho final nunca passa de `1 / pico`: dentro desse teto,
//! nunca corta o topo do sinal.

use std::path::Path;

use crate::Result;
use crate::decode::TrackDecoder;

/// Nível de referência do RMS, em dBFS. Não é o padrão de nenhuma norma —é
/// só o ponto médio que o ganho tenta alcançar. Mudar este número desloca o
/// volume de toda a biblioteca igualmente, então não faz diferença relativa
/// nenhuma; está aqui porque alguma referência precisa existir.
const REFERENCE_RMS_DBFS: f32 = -20.0;

/// Abaixo disso, o ganho vira alucinação numérica: um trecho quase mudo
/// (fade-out, silêncio de abertura) mediria RMS próximo de zero e pediria um
/// ganho absurdo. O piso evita `log10(0)`.
const FLOOR: f32 = 1.0e-6;

/// Ganho final nunca amplifica mais que isto (+12 dB) nem atenua mais que
/// isto (−20 dB). Sem teto, uma faixa quase silenciosa inteira (uma pista
/// de efeitos, um interlúdio falado) pediria dezenas de dB de ganho.
const MAX_GAIN: f32 = 4.0;
const MIN_GAIN: f32 = 0.1;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Loudness {
    /// Ganho sugerido pelo RMS, em dB — antes do teto do pico.
    pub gain_db: f32,
    /// Maior amostra absoluta vista na faixa inteira, em `[0, 1]`.
    pub peak: f32,
}

/// Decodifica `path` inteiro e mede RMS e pico.
///
/// Roda numa tarefa de fundo, nunca no caminho de tocar: decodificar o
/// arquivo todo custa o mesmo que tocá-lo, e o usuário não pode esperar isso
/// antes do som começar.
pub fn analyze(path: &Path) -> Result<Loudness> {
    let mut decoder = TrackDecoder::open(path)?;

    let mut sum_sq = 0.0f64;
    let mut count = 0u64;
    let mut peak = 0.0f32;

    while decoder.decode_next()? {
        for &sample in decoder.samples() {
            sum_sq += f64::from(sample) * f64::from(sample);
            count += 1;
            peak = peak.max(sample.abs());
        }
    }

    if count == 0 {
        // Arquivo sem samples (ex.: só metadata) — nem-liga nem-baixa.
        return Ok(Loudness {
            gain_db: 0.0,
            peak: 1.0,
        });
    }

    let rms = ((sum_sq / count as f64).sqrt() as f32).max(FLOOR);
    let rms_dbfs = 20.0 * rms.log10();

    Ok(Loudness {
        gain_db: REFERENCE_RMS_DBFS - rms_dbfs,
        peak: peak.max(FLOOR),
    })
}

/// Ganho linear a aplicar na hora de tocar, já com o teto do pico e os
/// limites de segurança — o único número que o motor de áudio precisa.
#[must_use]
pub fn linear_gain(loudness: Loudness) -> f32 {
    let from_rms = 10f32.powf(loudness.gain_db / 20.0);
    let ceiling = 1.0 / loudness.peak;
    from_rms.min(ceiling).clamp(MIN_GAIN, MAX_GAIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Write as _;

    /// Grava um WAV PCM de 16 bits, mono, com as amostras dadas.
    ///
    /// WAV em vez de MP3 aqui é deliberado: dá controle exato sobre a
    /// amplitude decodificada, e o MP3 "silencioso" usado no resto do
    /// workspace decodifica sempre em zero, não serviria para testar RMS de
    /// verdade.
    fn write_wav(path: &std::path::Path, samples: &[i16], sample_rate: u32) {
        let data_len = samples.len() * 2;
        let mut buf = Vec::with_capacity(44 + data_len);

        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
        buf.extend_from_slice(b"WAVE");

        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
        buf.extend_from_slice(&1u16.to_le_bytes()); // mono
        buf.extend_from_slice(&sample_rate.to_le_bytes());
        buf.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
        buf.extend_from_slice(&2u16.to_le_bytes()); // block align
        buf.extend_from_slice(&16u16.to_le_bytes()); // bits/sample

        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&(data_len as u32).to_le_bytes());
        for &s in samples {
            buf.extend_from_slice(&s.to_le_bytes());
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("criar diretório");
        }
        let mut file = std::fs::File::create(path).expect("criar wav");
        file.write_all(&buf).expect("gravar wav");
    }

    fn tmp_wav(nome: &str, samples: &[i16]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("player-loudness-{nome}.wav"));
        write_wav(&path, samples, 44100);
        path
    }

    #[test]
    fn sinal_constante_tem_rms_igual_ao_pico() {
        // DC em metade da escala: toda amostra igual, então RMS == pico.
        let samples = vec![16384i16; 4410]; // 0,1 s
        let path = tmp_wav("dc", &samples);

        let loudness = analyze(&path).expect("analisar");
        assert!(
            (loudness.peak - 0.5).abs() < 0.01,
            "pico esperado ~0.5, veio {}",
            loudness.peak
        );
    }

    #[test]
    fn faixa_mais_baixa_recebe_mais_ganho_que_faixa_mais_alta() {
        let baixa = tmp_wav("baixa", &vec![3277i16; 4410]); // ~0.1 de amplitude
        let alta = tmp_wav("alta", &vec![26214i16; 4410]); // ~0.8 de amplitude

        let ganho_baixa = analyze(&baixa).expect("analisar baixa").gain_db;
        let ganho_alta = analyze(&alta).expect("analisar alta").gain_db;

        assert!(
            ganho_baixa > ganho_alta,
            "faixa baixa (ganho {ganho_baixa}) devia pedir mais ganho que a alta ({ganho_alta})"
        );
    }

    #[test]
    fn silencio_nao_produz_ganho_infinito_ou_nan() {
        let path = tmp_wav("silencio", &vec![0i16; 4410]);
        let loudness = analyze(&path).expect("analisar silêncio");

        assert!(loudness.gain_db.is_finite());
        let gain = linear_gain(loudness);
        assert!(gain.is_finite());
        assert!((MIN_GAIN..=MAX_GAIN).contains(&gain));
    }

    /// O caso que justifica levar o pico em conta: RMS baixo pediria ganho
    /// alto, mas um pico já perto do teto tem que travar o ganho antes de
    /// estourar 0 dBFS.
    #[test]
    fn pico_alto_trava_o_ganho_mesmo_com_rms_baixo() {
        // Maioria das amostras bem baixas (RMS baixo), uma rajada no teto.
        let mut samples = vec![500i16; 4410];
        for s in samples.iter_mut().step_by(50) {
            *s = 32000;
        }
        let path = tmp_wav("pico", &samples);

        let loudness = analyze(&path).expect("analisar");
        let gain = linear_gain(loudness);

        assert!(
            gain * loudness.peak <= 1.01,
            "ganho {gain} com pico {} estouraria 0 dBFS",
            loudness.peak
        );
    }

    #[test]
    fn ganho_nunca_passa_dos_limites_de_seguranca() {
        // RMS altíssimo pedido (faixa "alta" de propósito) não deve nunca
        // resultar em atenuação além do mínimo nem amplificação além do
        // máximo, não importa a extremidade.
        for db in [-80.0, -40.0, 0.0, 40.0, 80.0] {
            let gain = linear_gain(Loudness {
                gain_db: db,
                peak: 1.0,
            });
            assert!(
                (MIN_GAIN..=MAX_GAIN).contains(&gain),
                "gain_db={db} produziu ganho fora da faixa: {gain}"
            );
        }
    }
}
