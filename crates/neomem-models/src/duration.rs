use std::time::Duration;

pub fn parse(text: &str) -> Option<Duration> {
    let trimmed = text.trim();
    let split = trimmed.find(|character: char| character.is_ascii_alphabetic())?;
    let (number, unit) = trimmed.split_at(split);
    let amount: f64 = number.trim().parse().ok()?;
    let seconds = match unit {
        "ms" => amount / 1000.0,
        "s" => amount,
        "m" => amount * 60.0,
        "h" => amount * 3600.0,
        _ => return None,
    };
    Duration::try_from_secs_f64(seconds).ok()
}

#[cfg(test)]
mod tests {
    use super::parse;
    use std::time::Duration;

    #[test]
    fn reads_the_units_the_config_uses() {
        assert_eq!(parse("2.5s"), Some(Duration::from_millis(2500)));
        assert_eq!(parse("15m"), Some(Duration::from_mins(15)));
        assert_eq!(parse("600ms"), Some(Duration::from_millis(600)));
        assert_eq!(parse("soon"), None);
    }
}
