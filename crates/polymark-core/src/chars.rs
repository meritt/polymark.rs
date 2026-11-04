pub use crate::signs::is_sign;

pub fn utf16_len(value: &str) -> usize {
    value.chars().map(char::len_utf16).sum()
}

pub fn is_space(char: char) -> bool {
    matches!(
        char,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

pub fn trim(value: &str) -> &str {
    value.trim_matches(is_space)
}

pub fn is_blank(value: &str) -> bool {
    trim(value).is_empty()
}

pub fn squeeze(value: &str) -> String {
    let mut result = String::with_capacity(value.len());

    for word in value.split(is_space).filter(|word| !word.is_empty()) {
        if !result.is_empty() {
            result.push(' ');
        }

        result.push_str(word);
    }

    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Space,
    Punctuation,
    Word,
}

pub fn kind(char: Option<char>) -> Kind {
    match char {
        None => Kind::Punctuation,
        Some(char) if is_space(char) => Kind::Space,
        Some(char) if is_sign(char) => Kind::Punctuation,
        Some(_) => Kind::Word,
    }
}

pub fn number(value: &str) -> f64 {
    let value = trim(value);

    if value.is_empty() {
        return 0.0;
    }

    let radix = [("0x", 16), ("0X", 16), ("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)]
        .into_iter()
        .find_map(|(prefix, radix)| value.strip_prefix(prefix).map(|digits| (digits, radix)));

    if let Some((digits, radix)) = radix {
        let value = digits
            .chars()
            .try_fold(0.0, |total, char| char.to_digit(radix).map(|digit| total * f64::from(radix) + f64::from(digit)));

        return if digits.is_empty() { f64::NAN } else { value.unwrap_or(f64::NAN) };
    }

    let finite = value.bytes().all(|byte| byte.is_ascii_digit() || b"+-.eE".contains(&byte));

    if !finite {
        return match value {
            "Infinity" | "+Infinity" => f64::INFINITY,
            "-Infinity" => f64::NEG_INFINITY,
            _ => f64::NAN,
        };
    }

    value.parse().unwrap_or(f64::NAN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes() {
        assert!(is_space('\u{a0}'));
        assert!(!is_space('\u{200b}'));
        assert!(is_sign('«'));
        assert!(is_sign('$'));
        assert!(!is_sign('я'));
        assert_eq!(utf16_len("😀a"), 3);
        assert_eq!(squeeze("  a \n b  "), "a b");
    }

    #[test]
    fn numbers() {
        assert!((number(" 5 ") - 5.0).abs() < f64::EPSILON);
        assert!((number("0x10") - 16.0).abs() < f64::EPSILON);
        assert!(number("").abs() < f64::EPSILON);
        assert!(number("5a").is_nan());
    }
}
