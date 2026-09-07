//! Gera uma biblioteca de música sintética para medir scan, índice e memória.
//!
//! Otimização sem medição é chute, e não dá pra descobrir problema de
//! performance com 200 arquivos: os gargalos de uma biblioteca de 50k faixas
//! (travessia do diretório, parse de tag, dedupe de capa, pressão de RAM) só
//! aparecem na escala certa.
//!
//! Os arquivos são MP3 **válidos** — frames de silêncio de verdade, com tag
//! ID3v2 e capa embutida — então `lofty` e `symphonia` fazem exatamente o
//! mesmo trabalho que fariam numa biblioteca real.
//!
//! ```text
//! cargo run --release -p libgen -- --out ./testdata/lib50k --tracks 50000
//! ```

mod png;
mod rng;

use std::collections::HashSet;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use lofty::config::WriteOptions;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::tag::{Accessor, ItemKey, Tag, TagExt, TagType};

use rng::Rng;

/// Erro capaz de atravessar `thread::scope`. `Box<dyn Error>` sozinho não é
/// `Send`, então não sai de uma thread de trabalho.
type ThreadError = Box<dyn Error + Send + Sync>;

// -----------------------------------------------------------------------------
// MP3
// -----------------------------------------------------------------------------

/// MPEG-1 Layer III, 128 kbps, 44,1 kHz, estéreo, sem CRC.
const FRAME_HEADER: [u8; 4] = [0xFF, 0xFB, 0x90, 0x00];
/// `144 * 128000 / 44100`, truncado.
const FRAME_LEN: usize = 417;
/// Um frame Layer III carrega 1152 samples.
const SAMPLES_PER_FRAME: u32 = 1152;
const SAMPLE_RATE: u32 = 44100;

/// Monta um MP3 de silêncio com a duração pedida.
///
/// O corpo do frame é todo zero: `part2_3_length` zerado significa "nenhum
/// dado de áudio", que todo decodificador lê como silêncio. É o menor arquivo
/// que ainda exercita o parser de MP3 de verdade.
fn silent_mp3(seconds: u32) -> Vec<u8> {
    let frames = (seconds * SAMPLE_RATE).div_ceil(SAMPLES_PER_FRAME) as usize;
    let mut out = vec![0u8; frames * FRAME_LEN];
    for frame in out.as_chunks_mut::<FRAME_LEN>().0 {
        frame[..4].copy_from_slice(&FRAME_HEADER);
    }
    out
}

// -----------------------------------------------------------------------------
// Nomes
// -----------------------------------------------------------------------------

// Acento e caixa misturada de propósito: é o que valida o dobramento de chave
// do `player-core::norm` e o `remove_diacritics` do FTS5.
const ADJ: &[&str] = &[
    "Último",
    "Azul",
    "Silent",
    "Órbita",
    "Elétrico",
    "Vermelho",
    "Distant",
    "Solar",
    "Ínfimo",
    "Nocturnal",
    "Vazio",
    "Golden",
    "Áspero",
    "Quiet",
];
const NOUN: &[&str] = &[
    "Verão",
    "Machine",
    "Coração",
    "Signal",
    "Estação",
    "Mirror",
    "Água",
    "Cavalo",
    "Tempest",
    "Sertão",
    "Circuit",
    "Névoa",
    "Harbour",
    "Ilhas",
];
const GENRE: &[&str] = &[
    "Rock",
    "MPB",
    "Jazz",
    "Electronic",
    "Ambient",
    "Samba",
    "Post-Punk",
];

fn two_words(rng: &mut Rng) -> String {
    format!("{} {}", rng.pick(ADJ), rng.pick(NOUN))
}

/// Tira o que quebra em algum sistema de arquivos. Os nomes gerados já são
/// seguros; isto é rede de proteção pra quando a lista crescer.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if "/\\:*?\"<>|".contains(c) { '_' } else { c })
        .collect()
}

// -----------------------------------------------------------------------------
// Plano
// -----------------------------------------------------------------------------

struct AlbumPlan {
    artist: String,
    album: String,
    genre: &'static str,
    year: u32,
    /// Uma capa por álbum, repetida em todas as faixas — é o caso que a
    /// deduplicação por hash tem que colapsar.
    art: Vec<u8>,
    titles: Vec<String>,
}

/// Garante que `base` não colida com nada já usado, sufixando se preciso.
///
/// Sem isso o corpus fica silenciosamente menor do que o pedido: o pool de
/// nomes é pequeno, dois artistas sorteiam o mesmo nome, caem no mesmo
/// diretório e as faixas de mesmo número se sobrescrevem. Contar 50 000
/// escritas e encontrar 49 978 arquivos estraga qualquer medição.
fn unique(used: &mut HashSet<String>, base: String) -> String {
    if used.insert(base.clone()) {
        return base;
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{base} {n}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

/// Monta a biblioteca inteira em memória antes de escrever nada.
///
/// Custa alguns MB e paga: o plano é determinístico e independente de quantas
/// threads escrevem depois, então o corpus não muda entre execuções.
fn plan(tracks: usize, seed: u64, art_px: u32) -> Vec<AlbumPlan> {
    let mut rng = Rng::new(seed);
    let mut albums = Vec::new();
    let mut remaining = tracks;
    let mut used_artists = HashSet::new();

    while remaining > 0 {
        let artist = unique(&mut used_artists, sanitize(&two_words(&mut rng)));
        let mut used_albums = HashSet::new();

        // Um artista com vários álbuns é o normal, e é o que faz a árvore de
        // diretórios ter a forma de uma biblioteca de verdade.
        for _ in 0..rng.range(1, 4) {
            if remaining == 0 {
                break;
            }
            let n = rng.range(6, 14).min(remaining);
            remaining -= n;

            albums.push(AlbumPlan {
                artist: artist.clone(),
                album: unique(&mut used_albums, sanitize(&two_words(&mut rng))),
                genre: rng.pick(GENRE),
                year: 1970 + rng.below(55) as u32,
                art: png::cover(art_px, rng.next_u64()),
                titles: (0..n).map(|_| sanitize(&two_words(&mut rng))).collect(),
            });
        }
    }

    albums
}

// -----------------------------------------------------------------------------
// Escrita
// -----------------------------------------------------------------------------

fn write_album(
    root: &Path,
    plan: &AlbumPlan,
    audio: &[u8],
    done: &AtomicUsize,
) -> Result<(), ThreadError> {
    let dir = root.join(&plan.artist).join(&plan.album);
    fs::create_dir_all(&dir)?;

    for (i, title) in plan.titles.iter().enumerate() {
        let no = u32::try_from(i + 1)?;
        let path = dir.join(format!("{no:02} - {title}.mp3"));

        // Primeiro o áudio; a tag entra por cima, como num arquivo real.
        fs::write(&path, audio)?;

        let mut tag = Tag::new(TagType::Id3v2);
        tag.set_title(title.clone());
        tag.set_artist(plan.artist.clone());
        tag.set_album(plan.album.clone());
        tag.set_genre(plan.genre.to_string());
        tag.set_year(plan.year);
        tag.set_track(no);
        tag.set_disk(1);
        tag.insert_text(ItemKey::AlbumArtist, plan.artist.clone());
        tag.push_picture(Picture::new_unchecked(
            PictureType::CoverFront,
            Some(MimeType::Png),
            None,
            plan.art.clone(),
        ));
        tag.save_to_path(&path, WriteOptions::default())?;

        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
        if n.is_multiple_of(2000) {
            println!("  {n} faixas…");
        }
    }

    Ok(())
}

// -----------------------------------------------------------------------------
// CLI
// -----------------------------------------------------------------------------

struct Args {
    out: PathBuf,
    tracks: usize,
    seconds: u32,
    seed: u64,
    art_px: u32,
    threads: usize,
}

const USAGE: &str = "\
uso: libgen --out <dir> [opções]

  --out <dir>       onde criar a biblioteca (obrigatório)
  --tracks <n>      número de faixas            [1000]
  --seconds <n>     duração de cada faixa       [1]
  --seed <n>        semente; mesma seed = mesma biblioteca [1]
  --art <px>        lado da capa embutida       [32]
  --threads <n>     escritores em paralelo      [núcleos]

Cada faixa custa ~16 KB por segundo de áudio, mais a capa.
50k faixas com o padrão dão ~1 GB.";

fn parse_args() -> Result<Args, ThreadError> {
    let mut out = None;
    let mut tracks = 1000usize;
    let mut seconds = 1u32;
    let mut seed = 1u64;
    let mut art_px = 32u32;
    let mut threads = std::thread::available_parallelism().map_or(4, Into::into);

    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| format!("faltou o valor de {flag}"))
        };
        match flag.as_str() {
            "--out" => out = Some(PathBuf::from(value()?)),
            "--tracks" => tracks = value()?.parse()?,
            "--seconds" => seconds = value()?.parse()?,
            "--seed" => seed = value()?.parse()?,
            "--art" => art_px = value()?.parse()?,
            "--threads" => threads = value()?.parse()?,
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(format!("opção desconhecida: {other}\n\n{USAGE}").into()),
        }
    }

    Ok(Args {
        out: out.ok_or_else(|| format!("--out é obrigatório\n\n{USAGE}"))?,
        tracks,
        seconds,
        seed,
        art_px,
        threads: threads.max(1),
    })
}

fn main() -> Result<(), ThreadError> {
    let args = parse_args()?;
    let started = Instant::now();

    println!("planejando {} faixas (seed {})…", args.tracks, args.seed);
    let albums = plan(args.tracks, args.seed, args.art_px);

    // Um único buffer de áudio compartilhado por todas as faixas: o conteúdo
    // sonoro é idêntico mesmo, e alocar 50k cópias de 16 KB não teria graça.
    let audio = silent_mp3(args.seconds);

    fs::create_dir_all(&args.out)?;
    println!(
        "escrevendo {} álbuns em {} com {} threads…",
        albums.len(),
        args.out.display(),
        args.threads
    );

    let done = AtomicUsize::new(0);
    let chunk = albums.len().div_ceil(args.threads);

    std::thread::scope(|scope| -> Result<(), ThreadError> {
        let mut workers = Vec::with_capacity(args.threads);
        for slice in albums.chunks(chunk.max(1)) {
            let (out, audio, done) = (&args.out, &audio, &done);
            workers.push(scope.spawn(move || {
                for album in slice {
                    write_album(out, album, audio, done)?;
                }
                Ok::<_, ThreadError>(())
            }));
        }
        for worker in workers {
            worker
                .join()
                .map_err(|_| "thread de escrita entrou em pânico")??;
        }
        Ok(())
    })?;

    let total = done.load(Ordering::Relaxed);
    let elapsed = started.elapsed();
    println!(
        "pronto: {total} faixas em {} álbuns, {:.1}s ({:.0} faixas/s)",
        albums.len(),
        elapsed.as_secs_f64(),
        total as f64 / elapsed.as_secs_f64()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mp3_tem_frames_inteiros_e_duracao_certa() {
        let data = silent_mp3(2);
        assert!(data.len().is_multiple_of(FRAME_LEN));

        let frames = data.len() / FRAME_LEN;
        let secs = (frames as f64 * f64::from(SAMPLES_PER_FRAME)) / f64::from(SAMPLE_RATE);
        assert!(
            (secs - 2.0).abs() < 0.03,
            "duração fora do esperado: {secs}"
        );
    }

    #[test]
    fn todo_frame_comeca_com_sync() {
        let data = silent_mp3(1);
        for frame in data.as_chunks::<FRAME_LEN>().0 {
            assert_eq!(&frame[..4], &FRAME_HEADER);
        }
    }

    #[test]
    fn plano_e_deterministico() {
        let a = plan(200, 7, 16);
        let b = plan(200, 7, 16);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(x.artist, y.artist);
            assert_eq!(x.album, y.album);
            assert_eq!(x.titles, y.titles);
            assert_eq!(x.art, y.art);
        }
    }

    #[test]
    fn plano_entrega_o_numero_pedido_de_faixas() {
        let total: usize = plan(1234, 3, 16).iter().map(|a| a.titles.len()).sum();
        assert_eq!(total, 1234);
    }

    /// Cada `(artista, álbum)` vira um diretório. Dois planos no mesmo
    /// diretório significam faixas sobrescritas e corpus menor que o pedido.
    #[test]
    fn nenhum_par_artista_album_se_repete() {
        let albums = plan(50_000, 1, 8);
        let mut seen = HashSet::new();
        for a in &albums {
            assert!(
                seen.insert((a.artist.clone(), a.album.clone())),
                "diretório duplicado: {}/{}",
                a.artist,
                a.album
            );
        }
    }

    #[test]
    fn capa_e_unica_por_album_e_repetida_dentro_dele() {
        let albums = plan(300, 11, 16);
        assert!(albums.len() > 2);
        assert_ne!(albums[0].art, albums[1].art);
    }
}
