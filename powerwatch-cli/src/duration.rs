use std::time::Duration;

pub fn parse_duration(text: &str) -> Result<Duration, String> {
    let text = text.trim();

    let (number_part, unit) = match text.chars().last() {
        Some(c) if c.is_ascii_alphabetic() => (&text[..text.len() - 1], c),
        _ => (text, 's'),
    };

    let number: u64 = number_part
        .parse()
        .map_err(|_| format!("invalid duration: {text}"))?;

    let seconds = match unit {
        's' => number,
        'm' => number * 60,
        'h' => number * 3600,
        other => return Err(format!("unknown duration unit '{other}' in {text}")),
    };

    Ok(Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_plain_number_of_seconds() {
        assert_eq!(parse_duration("90").unwrap(), Duration::from_secs(90));
    }

    #[test]
    fn parses_a_seconds_suffix() {
        assert_eq!(parse_duration("60s").unwrap(), Duration::from_secs(60));
    }

    #[test]
    fn parses_a_minutes_suffix() {
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
    }

    #[test]
    fn parses_an_hours_suffix() {
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
    }

    #[test]
    fn rejects_an_unknown_unit() {
        assert!(parse_duration("10x").is_err());
    }

    #[test]
    fn rejects_non_numeric_input() {
        assert!(parse_duration("soon").is_err());
    }
}
