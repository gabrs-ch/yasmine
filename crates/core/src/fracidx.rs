//! Índices fracionários para ordenar itens de playlist.
//!
//! A posição de um item é uma **string**, não um número, e a ordem da playlist
//! é a ordem lexicográfica dessas strings. Entre duas posições quaisquer
//! sempre cabe outra, então:
//!
//! - arrastar um item para o meio escreve **uma** linha, em vez de renumerar a
//!   playlist inteira;
//! - dois devices reordenando ao mesmo tempo geram posições independentes, e o
//!   merge do sync não precisa decidir nada.
//!
//! # Forma da chave
//!
//! Uma chave é `<parte inteira><fração>`, com dígitos `0-9A-Za-z` — que já
//! estão em ordem ASCII crescente, e é por isso que comparar as strings byte a
//! byte dá a ordem numérica de graça.
//!
//! A primeira letra da parte inteira diz o tamanho dela: `a` → 1 dígito,
//! `b` → 2, até `z` → 26, e `Z` → 1, `Y` → 2, até `A` → 26 do lado negativo.
//! Essa letra é o que mantém as chaves curtas: acrescentar no fim incrementa a
//! parte inteira (`a0`, `a1`, … `az`, `b00`, …), então **50 000 faixas
//! acrescentadas em sequência cabem em 4 bytes por chave**.
//!
//! Sem a parte inteira — bisseccionando sempre em direção a 1 — cada ~6
//! acréscimos custariam um byte a mais, e adicionar uma biblioteca de 50 000 a
//! uma playlist geraria dezenas de MB só de posições. Daí a máquina extra.
//!
//! Só inserir repetidamente **dentro do mesmo intervalo** alonga a chave, e aí
//! é inerente: o intervalo encolhe pela metade a cada vez.
//!
//! # Antes da Fase 4
//!
//! [`between`] é determinístico: dois devices inserindo no mesmo ponto geram a
//! **mesma** chave, e a chave primária `(playlist_id, position)` colidiria no
//! merge. Quando o sync entrar, isso se resolve acrescentando alguns dígitos
//! aleatórios ao fim da chave — o que preserva a ordem, porque a chave gerada
//! já é estritamente menor que `before` em algum dígito.

const DIGITS: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const BASE: usize = 62;

/// Primeira chave de uma lista vazia.
const FIRST: &str = "a0";
/// Menor parte inteira possível: abaixo dela não há para onde ir.
const SMALLEST_INTEGER: &str = "A00000000000000000000000000";

fn digit_of(byte: u8) -> usize {
    DIGITS.iter().position(|d| *d == byte).unwrap_or(0)
}

fn to_key(digits: &[usize]) -> String {
    digits
        .iter()
        .map(|&d| DIGITS[d.min(BASE - 1)] as char)
        .collect()
}

/// Quantos caracteres a parte inteira ocupa, a partir da letra inicial.
fn integer_len(head: u8) -> usize {
    match head {
        b'a'..=b'z' => (head - b'a') as usize + 2,
        b'A'..=b'Z' => (b'Z' - head) as usize + 2,
        // Chave malformada (só chega aqui vinda de fora): trata como mínima.
        _ => 2,
    }
}

fn integer_part(key: &str) -> &str {
    let len = key.as_bytes().first().map_or(2, |&h| integer_len(h));
    &key[..len.min(key.len())]
}

/// Próxima parte inteira. `None` quando não há mais espaço acima.
fn increment_integer(x: &str) -> Option<String> {
    let bytes = x.as_bytes();
    let head = *bytes.first()?;
    let mut digits: Vec<usize> = bytes[1..].iter().map(|b| digit_of(*b)).collect();

    let mut carry = true;
    for slot in digits.iter_mut().rev() {
        if !carry {
            break;
        }
        if *slot + 1 == BASE {
            *slot = 0;
        } else {
            *slot += 1;
            carry = false;
        }
    }

    if !carry {
        return Some(format!("{}{}", head as char, to_key(&digits)));
    }

    // Estourou: sobe de magnitude.
    match head {
        b'Z' => Some(format!("a{}", DIGITS[0] as char)),
        b'z' => None,
        _ => {
            let next = head + 1;
            // Do lado negativo a magnitude encurta; do positivo, alonga.
            if next > b'a' {
                digits.push(0);
            } else {
                digits.pop();
            }
            Some(format!("{}{}", next as char, to_key(&digits)))
        }
    }
}

/// Parte inteira anterior. `None` quando não há mais espaço abaixo.
fn decrement_integer(x: &str) -> Option<String> {
    let bytes = x.as_bytes();
    let head = *bytes.first()?;
    let mut digits: Vec<usize> = bytes[1..].iter().map(|b| digit_of(*b)).collect();

    let mut borrow = true;
    for slot in digits.iter_mut().rev() {
        if !borrow {
            break;
        }
        if *slot == 0 {
            *slot = BASE - 1;
        } else {
            *slot -= 1;
            borrow = false;
        }
    }

    if !borrow {
        return Some(format!("{}{}", head as char, to_key(&digits)));
    }

    match head {
        b'a' => Some(format!("Z{}", DIGITS[BASE - 1] as char)),
        b'A' => None,
        _ => {
            let next = head - 1;
            if next < b'Z' {
                digits.push(BASE - 1);
            } else {
                digits.pop();
            }
            Some(format!("{}{}", next as char, to_key(&digits)))
        }
    }
}

/// Fração estritamente entre `a` e `b`, ambas sem parte inteira.
///
/// `b = None` significa 1. Exige `a < b` e que nenhuma das duas termine em
/// `'0'` — invariantes que [`between`] mantém.
fn midpoint(a: &str, b: Option<&str>) -> String {
    if let Some(b) = b {
        // Copia o prefixo em comum e resolve o resto.
        let common = a
            .bytes()
            .chain(std::iter::repeat(b'0'))
            .zip(b.bytes())
            .take_while(|(x, y)| x == y)
            .count();
        if common > 0 {
            let tail_a = a.get(common..).unwrap_or("");
            return format!("{}{}", &b[..common], midpoint(tail_a, Some(&b[common..])));
        }
    }

    let digit_a = a.as_bytes().first().map_or(0, |b| digit_of(*b));
    let digit_b = b
        .and_then(|b| b.as_bytes().first().copied())
        .map_or(BASE, digit_of);

    if digit_b > digit_a + 1 {
        // Sobra espaço: o ponto médio arredondado resolve.
        return (DIGITS[(digit_a + digit_b).div_ceil(2)] as char).to_string();
    }

    match b {
        // `b` tem mais dígitos: o primeiro dele já é maior que `a`.
        Some(b) if b.len() > 1 => b[..1].to_owned(),
        // Dígitos colados: fixa o de `a` e desce um nível.
        _ => {
            let head = a.as_bytes().first().map_or('0', |&b| b as char);
            format!("{head}{}", midpoint(a.get(1..).unwrap_or(""), None))
        }
    }
}

/// Uma posição estritamente entre `after` e `before`.
///
/// `after = None` significa "antes de tudo" e `before = None`, "depois de
/// tudo". Com os dois `None`, devolve a primeira posição de uma lista vazia.
///
/// Exige `after < before`; chamar com a ordem trocada devolve uma chave sem
/// garantia. A checagem fica com quem chama, que é quem conhece a lista.
#[must_use]
pub fn between(after: Option<&str>, before: Option<&str>) -> String {
    match (after, before) {
        (None, None) => FIRST.to_owned(),

        (None, Some(b)) => {
            let ib = integer_part(b);
            let fb = &b[ib.len()..];
            if ib == SMALLEST_INTEGER {
                // Não dá para baixar a magnitude: cabe na fração.
                return format!("{ib}{}", midpoint("", Some(fb)));
            }
            if ib < b {
                // `b` tem fração: a própria parte inteira já é menor.
                return ib.to_owned();
            }
            decrement_integer(ib).unwrap_or_else(|| format!("{ib}{}", midpoint("", Some(fb))))
        }

        (Some(a), None) => {
            let ia = integer_part(a);
            increment_integer(ia)
                .unwrap_or_else(|| format!("{ia}{}", midpoint(&a[ia.len()..], None)))
        }

        (Some(a), Some(b)) => {
            let ia = integer_part(a);
            let fa = &a[ia.len()..];
            let ib = integer_part(b);
            let fb = &b[ib.len()..];

            if ia == ib {
                return format!("{ia}{}", midpoint(fa, Some(fb)));
            }
            match increment_integer(ia) {
                // Se a próxima parte inteira já cabe antes de `b`, ela basta.
                Some(next) if next.as_str() < b => next,
                _ => format!("{ia}{}", midpoint(fa, None)),
            }
        }
    }
}

/// Posições para acrescentar `count` itens no fim de uma lista que termina em
/// `last`.
#[must_use]
pub fn append_many(last: Option<&str>, count: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(count);
    let mut previous = last.map(str::to_owned);
    for _ in 0..count {
        let next = between(previous.as_deref(), None);
        previous = Some(next.clone());
        out.push(next);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Valores conhecidos do algoritmo de referência. Se algum destes mudar, a
    /// implementação divergiu de todo mundo que fala o mesmo formato — e no
    /// sync isso significa duas playlists com ordens diferentes.
    #[test]
    fn casos_canonicos() {
        assert_eq!(between(None, None), "a0");
        assert_eq!(between(Some("a0"), None), "a1");
        assert_eq!(between(None, Some("a0")), "Zz");
        assert_eq!(between(Some("a0"), Some("a1")), "a0V");
        assert_eq!(between(Some("a0"), Some("a2")), "a1");
        assert_eq!(between(None, Some("Zz")), "Zy");
        assert_eq!(between(Some("Zz"), Some("a0")), "ZzV");
        // A virada de magnitude: depois de `az` vem `b00`, não `az` mais longo.
        assert_eq!(between(Some("az"), None), "b00");
    }

    #[test]
    fn cabe_sempre_no_meio() {
        let a = between(None, None);
        let b = between(Some(&a), None);
        let meio = between(Some(&a), Some(&b));
        assert!(a < meio && meio < b, "{a} < {meio} < {b}");
    }

    #[test]
    fn antes_do_primeiro() {
        let a = between(None, None);
        let antes = between(None, Some(&a));
        assert!(antes < a, "{antes} < {a}");
    }

    /// O ponto de ter parte inteira: acrescentar no fim não alonga a chave.
    #[test]
    fn cinquenta_mil_acrescimos_cabem_em_quatro_bytes() {
        let chaves = append_many(None, 50_000);
        let maior = chaves.iter().map(String::len).max().unwrap_or(0);
        assert!(maior <= 4, "chave de {maior} bytes acrescentando no fim");

        for par in chaves.windows(2) {
            assert!(par[0] < par[1], "saiu de ordem: {} < {}", par[0], par[1]);
        }
    }

    #[test]
    fn cinquenta_mil_acrescimos_sem_colisao() {
        let chaves = append_many(None, 50_000);
        let mut unicas = chaves.clone();
        unicas.sort();
        unicas.dedup();
        assert_eq!(unicas.len(), 50_000, "houve colisão");
        assert_eq!(
            chaves, unicas,
            "a ordem de geração não bate com a lexicográfica"
        );
    }

    /// O caso que alonga de verdade — e mesmo ele fica em tamanho utilizável.
    #[test]
    fn mil_insercoes_no_mesmo_lugar_continuam_ordenadas() {
        let a = between(None, None);
        let b = between(Some(&a), None);

        let mut baixo = a;
        for n in 0..1000 {
            let meio = between(Some(&baixo), Some(&b));
            assert!(
                baixo < meio && meio < b,
                "quebrou na inserção {n}: {baixo} < {meio} < {b}"
            );
            baixo = meio;
        }
        assert!(baixo.len() < 250, "chave de {} bytes", baixo.len());
    }

    #[test]
    fn insercoes_alternadas_nas_duas_pontas_continuam_ordenadas() {
        let mut chaves = vec![between(None, None)];
        for _ in 0..300 {
            let inicio = between(None, Some(&chaves[0]));
            chaves.insert(0, inicio);
            let fim = between(chaves.last().map(String::as_str), None);
            chaves.push(fim);
        }
        let mut ordenadas = chaves.clone();
        ordenadas.sort();
        assert_eq!(chaves, ordenadas);
    }

    /// Arrastar itens para posições aleatórias, muitas vezes, sem quebrar.
    #[test]
    fn mil_insercoes_em_pontos_aleatorios() {
        let mut chaves: Vec<String> = append_many(None, 20);
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;

        for _ in 0..1000 {
            seed ^= seed >> 12;
            seed ^= seed << 25;
            seed ^= seed >> 27;
            let at = (seed.wrapping_mul(0x2545_F491_4F6C_DD1D) as usize) % (chaves.len() + 1);

            let antes = at
                .checked_sub(1)
                .and_then(|i| chaves.get(i))
                .map(String::as_str);
            let depois = chaves.get(at).map(String::as_str);
            let nova = between(antes, depois);
            chaves.insert(at, nova);
        }

        for par in chaves.windows(2) {
            assert!(par[0] < par[1], "saiu de ordem: {} < {}", par[0], par[1]);
        }
    }

    #[test]
    fn append_many_continua_de_onde_a_lista_parou() {
        let inicio = append_many(None, 3);
        let resto = append_many(inicio.last().map(String::as_str), 3);
        assert!(inicio.last() < resto.first());
    }
}
