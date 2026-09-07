//! Normalização de texto para chaves de deduplicação.
//!
//! Só serve pra *agrupar* — nunca pra exibir. O nome que o usuário vê é sempre
//! o original da tag.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Dobra um nome numa chave estável: minúsculas, sem acento, espaços
/// colapsados, sem borda.
///
/// É o que impede "Legião Urbana", "legiao urbana" e "Legião  Urbana " de
/// virarem três artistas diferentes na biblioteca.
///
/// Decompõe em NFD e descarta os combining marks, então cobre qualquer acento
/// Unicode — não é uma tabela de latim-1.
#[must_use]
pub fn fold_key(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;

    for c in s.nfd() {
        if is_combining_mark(c) {
            continue;
        }
        if c.is_whitespace() {
            // Espaço só entra se vier texto depois: mata borda e sequência.
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.extend(c.to_lowercase());
    }

    out
}

/// Separador de campos dentro de uma chave composta. U+001F (unit separator)
/// não aparece em tag de música, então não precisa de escape.
const FIELD_SEP: char = '\u{1f}';

/// Chave única de um álbum. Precisa do artista junto: "Greatest Hits" existe
/// aos montes, e dois álbuns homônimos de artistas diferentes não podem
/// colapsar num só.
#[must_use]
pub fn album_key(album_artist: Option<&str>, title: &str) -> String {
    let artist = album_artist.map_or_else(String::new, fold_key);
    let mut key = String::with_capacity(artist.len() + title.len() + 1);
    key.push_str(&artist);
    key.push(FIELD_SEP);
    key.push_str(&fold_key(title));
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dobra_acento_caixa_e_espaco() {
        assert_eq!(fold_key("Legião Urbana"), "legiao urbana");
        assert_eq!(fold_key("  legiao   URBANA  "), "legiao urbana");
        assert_eq!(fold_key("Legião Urbana"), fold_key("legiao urbana"));
    }

    #[test]
    fn cobre_acento_fora_do_latim1() {
        assert_eq!(fold_key("Dvořák"), "dvorak");
        assert_eq!(fold_key("Sigur Rós"), "sigur ros");
    }

    #[test]
    fn album_homonimo_de_artistas_diferentes_nao_colide() {
        assert_ne!(
            album_key(Some("Queen"), "Greatest Hits"),
            album_key(Some("ABBA"), "Greatest Hits")
        );
    }

    #[test]
    fn album_sem_artista_ainda_gera_chave() {
        assert_eq!(album_key(None, "Bootleg"), "\u{1f}bootleg");
    }
}
