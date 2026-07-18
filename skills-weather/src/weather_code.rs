//! WMO weather-code → short French phrase mapping.
//!
//! Codes are documented at
//! <https://open-meteo.com/en/docs> (WMO Weather interpretation codes).

pub fn fr_phrase(code: i64) -> &'static str {
    match code {
        0 => "temps clair",
        1..=3 => "quelques nuages",
        45 | 48 => "du brouillard",
        51..=57 => "de la bruine",
        61..=65 => "de la pluie",
        66 | 67 => "de la pluie verglaçante",
        71..=75 => "de la neige",
        77 => "des grains de neige",
        80..=82 => "des averses",
        85 | 86 => "des averses de neige",
        95..=99 => "un orage",
        _ => "un temps particulier",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_sky() {
        assert_eq!(fr_phrase(0), "temps clair");
    }

    #[test]
    fn light_clouds_range() {
        for c in 1..=3 {
            assert_eq!(fr_phrase(c), "quelques nuages", "code {c}");
        }
    }

    #[test]
    fn fog_codes() {
        assert_eq!(fr_phrase(45), "du brouillard");
        assert_eq!(fr_phrase(48), "du brouillard");
    }

    #[test]
    fn drizzle_range() {
        for c in 51..=57 {
            assert_eq!(fr_phrase(c), "de la bruine", "code {c}");
        }
    }

    #[test]
    fn rain_range() {
        for c in 61..=65 {
            assert_eq!(fr_phrase(c), "de la pluie", "code {c}");
        }
    }

    #[test]
    fn freezing_rain() {
        assert_eq!(fr_phrase(66), "de la pluie verglaçante");
        assert_eq!(fr_phrase(67), "de la pluie verglaçante");
    }

    #[test]
    fn snow_range() {
        for c in 71..=75 {
            assert_eq!(fr_phrase(c), "de la neige", "code {c}");
        }
    }

    #[test]
    fn snow_grains() {
        assert_eq!(fr_phrase(77), "des grains de neige");
    }

    #[test]
    fn showers_range() {
        for c in 80..=82 {
            assert_eq!(fr_phrase(c), "des averses", "code {c}");
        }
    }

    #[test]
    fn snow_showers() {
        assert_eq!(fr_phrase(85), "des averses de neige");
        assert_eq!(fr_phrase(86), "des averses de neige");
    }

    #[test]
    fn thunderstorm_range() {
        for c in 95..=99 {
            assert_eq!(fr_phrase(c), "un orage", "code {c}");
        }
    }

    #[test]
    fn unknown_code_falls_back() {
        assert_eq!(fr_phrase(-1), "un temps particulier");
        assert_eq!(fr_phrase(42), "un temps particulier");
        assert_eq!(fr_phrase(200), "un temps particulier");
    }
}
