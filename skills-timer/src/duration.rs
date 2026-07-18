//! Parses a French duration phrase (e.g. `"deux secondes"`, `"5 minutes"`,
//! `"une heure"`) into a number of seconds.

/// Parses `text` into a number of seconds. Returns `None` if the phrase
/// isn't a recognized `<amount> <unit>` shape.
///
/// `amount` is either a bare integer (`"5"`) or one of the French number
/// words `un`/`une`..`dix` (1..10). `unit` is `seconde(s)`, `minute(s)`, or
/// `heure(s)`.
#[must_use]
pub fn parse_fr_duration(text: &str) -> Option<u64> {
    let mut words = text.split_whitespace();
    let amount_word = words.next()?;
    let unit_word = words.next()?;
    // Reject anything with extra trailing tokens.
    if words.next().is_some() {
        return None;
    }

    let amount = parse_amount(amount_word)?;
    let unit_seconds = parse_unit_seconds(unit_word)?;
    Some(amount * unit_seconds)
}

fn parse_amount(word: &str) -> Option<u64> {
    if let Ok(n) = word.parse::<u64>() {
        return Some(n);
    }
    let n = match word.to_lowercase().as_str() {
        "un" | "une" => 1,
        "deux" => 2,
        "trois" => 3,
        "quatre" => 4,
        "cinq" => 5,
        "six" => 6,
        "sept" => 7,
        "huit" => 8,
        "neuf" => 9,
        "dix" => 10,
        _ => return None,
    };
    Some(n)
}

fn parse_unit_seconds(word: &str) -> Option<u64> {
    match word.to_lowercase().as_str() {
        "seconde" | "secondes" => Some(1),
        "minute" | "minutes" => Some(60),
        "heure" | "heures" => Some(3600),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_digit_amounts() {
        assert_eq!(parse_fr_duration("5 minutes"), Some(300));
        assert_eq!(parse_fr_duration("2 secondes"), Some(2));
        assert_eq!(parse_fr_duration("1 heure"), Some(3600));
    }

    #[test]
    fn parses_word_amounts() {
        assert_eq!(parse_fr_duration("deux secondes"), Some(2));
        assert_eq!(parse_fr_duration("une heure"), Some(3600));
        assert_eq!(parse_fr_duration("un minute"), Some(60));
        assert_eq!(parse_fr_duration("trois minutes"), Some(180));
        assert_eq!(parse_fr_duration("quatre secondes"), Some(4));
        assert_eq!(parse_fr_duration("cinq secondes"), Some(5));
        assert_eq!(parse_fr_duration("six secondes"), Some(6));
        assert_eq!(parse_fr_duration("sept secondes"), Some(7));
        assert_eq!(parse_fr_duration("huit secondes"), Some(8));
        assert_eq!(parse_fr_duration("neuf secondes"), Some(9));
        assert_eq!(parse_fr_duration("dix secondes"), Some(10));
    }

    #[test]
    fn parses_all_units() {
        assert_eq!(parse_fr_duration("deux secondes"), Some(2));
        assert_eq!(parse_fr_duration("deux minutes"), Some(120));
        assert_eq!(parse_fr_duration("deux heures"), Some(7200));
    }

    #[test]
    fn rejects_unknown_words() {
        assert_eq!(parse_fr_duration("beaucoup de secondes"), None);
        assert_eq!(parse_fr_duration("deux jours"), None);
        assert_eq!(parse_fr_duration("deux"), None);
        assert_eq!(parse_fr_duration(""), None);
    }
}
