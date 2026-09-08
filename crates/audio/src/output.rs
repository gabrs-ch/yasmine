//! Saída de áudio: dispositivo `cpal` e o ring buffer que o alimenta.
//!
//! # A regra do callback
//!
//! O callback roda numa thread de tempo real do sistema. Ele **só copia
//! bytes**: nada de alocar, travar mutex, abrir arquivo ou logar. Qualquer uma
//! dessas coisas ali dentro vira estalo audível, porque o dispositivo não
//! espera — se o buffer não estiver pronto na hora, ele toca o que houver.
//!
//! Toda a comunicação com o resto do programa passa por átomos e por um ring
//! buffer lock-free (`rtrb`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::decode::Spec;
use crate::{Error, Result};

/// Estado compartilhado entre o callback de áudio, o worker e a UI.
///
/// Tudo atômico, nada de `Mutex`: a UI lê isso a cada frame e o callback
/// escreve em tempo real.
#[derive(Debug)]
pub struct Shared {
    /// Portão do callback: só é ligado quando o ring já tem áudio. Começar com
    /// o buffer vazio produz um underrun logo no primeiro callback.
    pub playing: AtomicBool,
    /// O que o usuário pediu. É isto que a UI mostra, para o botão não piscar
    /// "parado" durante os poucos milissegundos de pré-carga.
    pub intent: AtomicBool,
    /// Frames entregues ao dispositivo desde o início da faixa (ou desde o
    /// último seek). Dividido pela taxa, é a posição exata.
    pub frames_played: AtomicU64,
    pub sample_rate: AtomicU32,
    /// Duração da faixa em ms; 0 quando desconhecida.
    pub duration_ms: AtomicU64,
    /// Incrementado pelo worker para pedir que o callback jogue fora o que
    /// está no ring (seek ou troca de faixa).
    pub flush_gen: AtomicU64,
    /// Devolvido pelo callback quando o descarte terminou.
    pub flush_ack: AtomicU64,
    /// O callback pediu áudio e o ring estava vazio. Sintoma de worker lento
    /// ou ring curto demais.
    pub starved: AtomicBool,
    /// Volume mestre, ajustado pelo usuário. Bits de um `f32` em `[0, 1]`.
    pub master_volume: AtomicU32,
    /// Ganho do nivelador para a faixa que está tocando agora, já resolvido
    /// em linear (ver `loudness::linear_gain`). Bits de um `f32`. Troca no
    /// instante exato em que uma faixa emendada começa a soar de verdade —
    /// mesmo ponto em que `duration_ms` troca no `engine::Worker`.
    pub track_gain: AtomicU32,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            playing: AtomicBool::default(),
            intent: AtomicBool::default(),
            frames_played: AtomicU64::default(),
            sample_rate: AtomicU32::default(),
            duration_ms: AtomicU64::default(),
            flush_gen: AtomicU64::default(),
            flush_ack: AtomicU64::default(),
            starved: AtomicBool::default(),
            // Ganho neutro por padrão: sem isso, uma faixa tocaria muda até
            // a UI ou o worker terem a chance de fixar o valor de verdade.
            master_volume: AtomicU32::new(1.0f32.to_bits()),
            track_gain: AtomicU32::new(1.0f32.to_bits()),
        }
    }
}

/// Quanto áudio o ring segura. Meio segundo é folga suficiente para o worker
/// perder algumas fatias de CPU sem o som falhar, e curto o bastante para o
/// seek responder na hora.
const RING_SECONDS: f32 = 0.5;

/// Teto para o buffer de conversão do callback. Serve só para o `resize` não
/// acontecer dentro da thread de tempo real depois das primeiras chamadas.
const SCRATCH_FRAMES: usize = 8192;

pub struct Output {
    /// Precisa continuar viva: soltar o `Stream` fecha o dispositivo.
    _stream: cpal::Stream,
    pub producer: rtrb::Producer<f32>,
    /// O formato em que o dispositivo realmente abriu — pode não ser o pedido.
    pub spec: Spec,
}

impl Output {
    /// Abre o dispositivo padrão o mais perto possível de `want`.
    pub fn open(want: Spec, shared: &Arc<Shared>) -> Result<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(Error::NoDevice)?;
        let (config, format) = pick_config(&device, want)?;

        let channels = config.channels as usize;
        let spec = Spec {
            sample_rate: config.sample_rate.0,
            channels: config.channels,
        };

        let capacity = (spec.sample_rate as f32 * RING_SECONDS) as usize * channels;
        let (producer, consumer) = rtrb::RingBuffer::<f32>::new(capacity);

        let shared = Arc::clone(shared);
        let on_error = |err| eprintln!("erro no fluxo de áudio: {err}");

        let stream = match format {
            cpal::SampleFormat::F32 => device.build_output_stream(
                &config,
                callback::<f32>(consumer, shared, channels),
                on_error,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream(
                &config,
                callback::<i16>(consumer, shared, channels),
                on_error,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_output_stream(
                &config,
                callback::<u16>(consumer, shared, channels),
                on_error,
                None,
            ),
            other => {
                return Err(Error::UnsupportedFormat(format!(
                    "dispositivo só oferece amostras {other:?}"
                )));
            }
        }
        .map_err(|err| Error::Device(err.to_string()))?;

        stream
            .play()
            .map_err(|err| Error::Device(err.to_string()))?;

        Ok(Self {
            _stream: stream,
            producer,
            spec,
        })
    }
}

/// Monta o callback de áudio.
///
/// `T` é o tipo de amostra do dispositivo. Quando é `f32` a conversão final é
/// identidade e o compilador some com ela.
fn callback<T>(
    mut consumer: rtrb::Consumer<f32>,
    shared: Arc<Shared>,
    channels: usize,
) -> impl FnMut(&mut [T], &cpal::OutputCallbackInfo)
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let mut seen_gen = 0u64;
    // Alocado uma vez, aqui fora: dentro do callback não se aloca.
    let mut scratch = vec![0.0f32; SCRATCH_FRAMES * channels.max(1)];

    move |out, _info| {
        // 1. Descarte pedido pelo worker (seek ou troca de faixa). Mover o
        //    ponteiro de leitura é O(1) e não libera memória.
        let wanted = shared.flush_gen.load(Ordering::Acquire);
        if wanted != seen_gen {
            let slots = consumer.slots();
            if slots > 0
                && let Ok(chunk) = consumer.read_chunk(slots)
            {
                chunk.commit_all();
            }
            seen_gen = wanted;
            shared.flush_ack.store(wanted, Ordering::Release);
        }

        // 2. Pausado: silêncio, sem mexer no ring nem na posição.
        if !shared.playing.load(Ordering::Relaxed) {
            out.fill(T::EQUILIBRIUM);
            return;
        }

        if scratch.len() < out.len() {
            // Só acontece se o sistema entregar um buffer maior que o teto.
            scratch.resize(out.len(), 0.0);
        }

        // 3. Uma cópia em bloco, não sample a sample.
        let take = out.len().min(consumer.slots());
        if take > 0
            && let Ok(chunk) = consumer.read_chunk(take)
        {
            let (head, tail) = chunk.as_slices();
            scratch[..head.len()].copy_from_slice(head);
            scratch[head.len()..take].copy_from_slice(&tail[..take - head.len()]);
            chunk.commit_all();
        }

        // Um load cada, uma vez por callback — não por amostra. Combina
        // volume mestre e ganho do nivelador da faixa atual num só multiply.
        // O clamp final é rede de segurança, não o limitador de verdade: o
        // volume mestre nunca passa de 1 e o ganho da faixa já vem travado
        // no pico calculado (loudness::linear_gain); ele só entra em cena se
        // alguma dessas garantias falhar.
        let gain = f32::from_bits(shared.master_volume.load(Ordering::Relaxed))
            * f32::from_bits(shared.track_gain.load(Ordering::Relaxed));
        for (slot, sample) in out.iter_mut().zip(&scratch[..take]) {
            *slot = T::from_sample((*sample * gain).clamp(-1.0, 1.0));
        }
        // 4. Faltou áudio: completa com silêncio em vez de repetir o buffer
        //    anterior, que é o que produz o zumbido clássico de underrun.
        if take < out.len() {
            out[take..].fill(T::EQUILIBRIUM);
            shared.starved.store(true, Ordering::Relaxed);
        }

        if let Some(frames) = take.checked_div(channels) {
            shared
                .frames_played
                .fetch_add(frames as u64, Ordering::Relaxed);
        }
    }
}

/// Escolhe a configuração mais próxima do que o arquivo pede.
///
/// A prioridade é **a taxa do arquivo**: casando as duas, não existe
/// reamostragem, que é o ponto de ter escolhido `cpal` em vez de uma camada
/// mais alta. Número de canais vem depois, e o tipo de amostra por último.
fn pick_config(
    device: &cpal::Device,
    want: Spec,
) -> Result<(cpal::StreamConfig, cpal::SampleFormat)> {
    let supported: Vec<_> = device
        .supported_output_configs()
        .map_err(|err| Error::Device(err.to_string()))?
        .collect();

    if supported.is_empty() {
        return Err(Error::NoDevice);
    }

    let best = supported
        .iter()
        .min_by_key(|range| {
            let rate_ok = range.min_sample_rate().0 <= want.sample_rate
                && want.sample_rate <= range.max_sample_rate().0;
            let channel_penalty = if range.channels() == want.channels {
                0
            } else if range.channels() == 2 {
                1
            } else {
                2
            };
            let format_penalty = match range.sample_format() {
                cpal::SampleFormat::F32 => 0,
                cpal::SampleFormat::I16 => 1,
                _ => 2,
            };
            (u8::from(!rate_ok), channel_penalty, format_penalty)
        })
        .ok_or(Error::NoDevice)?;

    // Dentro da faixa suportada, fica na taxa do arquivo; fora dela, no limite
    // mais próximo.
    let sample_rate = want
        .sample_rate
        .clamp(best.min_sample_rate().0, best.max_sample_rate().0);

    Ok((
        cpal::StreamConfig {
            channels: best.channels(),
            sample_rate: cpal::SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Default,
        },
        best.sample_format(),
    ))
}
