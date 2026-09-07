//! Decodificação com `symphonia`.
//!
//! Entrega blocos de samples `f32` **intercalados**, que é a forma que o
//! dispositivo consome — converter aqui, no worker, é de graça; converter no
//! callback de áudio custaria estalo.

use std::fs::File;
use std::path::Path;
use std::time::Duration;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

use crate::{Error, Result};

/// Formato do áudio de uma faixa.
///
/// O `sample_rate` daqui é o que decide a taxa em que o dispositivo é aberto —
/// casando as duas, não existe reamostragem no caminho.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spec {
    pub sample_rate: u32,
    pub channels: u16,
}

pub struct TrackDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    spec: Spec,
    duration: Option<Duration>,
    /// Alocado no primeiro bloco, reusado em todos os seguintes.
    buf: Option<SampleBuffer<f32>>,
    len: usize,
}

impl TrackDecoder {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let stream = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());

        // A extensão é só uma dica; o `symphonia` confirma pelo conteúdo.
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let probed = symphonia::default::get_probe().format(
            &hint,
            stream,
            // `enable_gapless` descarta o delay e o padding que o encoder de
            // MP3 insere. Sem isso toda faixa começa com alguns
            // milissegundos de silêncio e o gapless nunca fecha de verdade.
            &FormatOptions {
                enable_gapless: true,
                ..FormatOptions::default()
            },
            &MetadataOptions::default(),
        )?;

        let format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or(Error::NoAudioTrack)?;

        let track_id = track.id;
        let params = track.codec_params.clone();
        let decoder = symphonia::default::get_codecs().make(&params, &DecoderOptions::default())?;

        let sample_rate = params
            .sample_rate
            .ok_or_else(|| Error::UnsupportedFormat("arquivo sem taxa de amostragem".into()))?;
        let channels = params
            .channels
            .map_or(2, |c| u16::try_from(c.count()).unwrap_or(2));

        let duration = params.n_frames.and_then(|frames| {
            params.time_base.map(|base| {
                let time = base.calc_time(frames);
                Duration::from_secs_f64(time.seconds as f64 + time.frac)
            })
        });

        Ok(Self {
            format,
            decoder,
            track_id,
            spec: Spec {
                sample_rate,
                channels,
            },
            duration,
            buf: None,
            len: 0,
        })
    }

    #[must_use]
    pub const fn spec(&self) -> Spec {
        self.spec
    }

    #[must_use]
    pub const fn duration(&self) -> Option<Duration> {
        self.duration
    }

    /// Decodifica o próximo bloco. `false` significa fim do arquivo.
    ///
    /// Devolver `bool` em vez do slice evita emprestar `self` dentro do laço,
    /// e deixa [`samples`](Self::samples) ser uma leitura barata.
    pub fn decode_next(&mut self) -> Result<bool> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                Err(SymError::IoError(err)) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                    self.len = 0;
                    return Ok(false);
                }
                Err(SymError::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(err) => return Err(err.into()),
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            match self.decoder.decode(&packet) {
                Ok(audio) => {
                    let buf = self.buf.get_or_insert_with(|| {
                        SampleBuffer::new(audio.capacity() as u64, *audio.spec())
                    });
                    buf.copy_interleaved_ref(audio);
                    self.len = buf.len();
                    if self.len == 0 {
                        continue;
                    }
                    return Ok(true);
                }
                // Um pacote corrompido no meio do arquivo não pode parar a
                // música: pula e segue.
                Err(SymError::DecodeError(_)) => continue,
                Err(err) => return Err(err.into()),
            }
        }
    }

    /// Samples intercalados do último bloco decodificado.
    #[must_use]
    pub fn samples(&self) -> &[f32] {
        self.buf
            .as_ref()
            .map_or(&[][..], |buf| &buf.samples()[..self.len])
    }

    pub fn seek(&mut self, position: Duration) -> Result<()> {
        self.format.seek(
            SeekMode::Accurate,
            SeekTo::Time {
                time: Time::from(position.as_secs_f64()),
                track_id: Some(self.track_id),
            },
        )?;
        // O decodificador guarda estado entre pacotes; sem reset, o primeiro
        // bloco depois do seek sai sujo.
        self.decoder.reset();
        self.len = 0;
        Ok(())
    }
}
