//! Isola o custo de materializar uma capa de tamanho realista.
//!
//! O corpus sintético do `libgen` embute capas minúsculas, senão 50 000
//! arquivos com uma capa de verdade dentro passariam de 10 GB. Isso deixa o
//! tempo de scan otimista: numa biblioteca real, cada álbum novo paga um
//! decode + resize + encode que o corpus não representa.
//!
//! Aqui esse custo é medido sozinho, sem tocar em disco de biblioteca.
//!
//! ```text
//! cargo run --release -p player-core --example artcost -- 1000 200
//! ```

use std::time::Instant;

use player_core::ArtCache;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let px: u32 = args.next().map_or(Ok(1000), |a| a.parse())?;
    let albums: usize = args.next().map_or(Ok(200), |a| a.parse())?;

    // Uma imagem com detalhe em cada pixel: capa lisa comprime demais e daria
    // um número bom demais para ser verdade.
    let mut img = image::RgbImage::new(px, px);
    for (x, y, pixel) in img.enumerate_pixels_mut() {
        *pixel = image::Rgb([
            (x ^ y) as u8,
            (x.wrapping_mul(7) ^ y.wrapping_mul(13)) as u8,
            (x.wrapping_add(y).wrapping_mul(3)) as u8,
        ]);
    }

    let mut blobs = Vec::with_capacity(albums);
    for n in 0..albums {
        // Um pixel diferente por álbum: hash distinto, custo de decode igual.
        let mut variant = img.clone();
        variant.put_pixel(0, 0, image::Rgb([(n & 0xFF) as u8, 0, 0]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(variant).write_to(
            &mut std::io::Cursor::new(&mut out),
            image::ImageFormat::Jpeg,
        )?;
        blobs.push(out);
    }

    let media_kb = blobs.iter().map(Vec::len).sum::<usize>() / albums / 1024;
    let cache = ArtCache::new(std::env::temp_dir().join("player-artcost"));

    let started = Instant::now();
    for blob in &blobs {
        cache.store(blob);
    }
    let elapsed = started.elapsed();

    let por_capa = elapsed.as_secs_f64() / albums as f64;
    println!("capa {px}x{px}, ~{media_kb} KB por blob, {albums} álbuns distintos");
    println!(
        "{:.2}s no total → {:.1} ms por capa",
        elapsed.as_secs_f64(),
        por_capa * 1000.0
    );
    println!(
        "extrapolando: 5 000 álbuns custariam ~{:.0}s de decode+resize no primeiro scan",
        por_capa * 5000.0
    );

    Ok(())
}
