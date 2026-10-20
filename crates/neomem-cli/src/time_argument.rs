use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use neomem_core::EpochMs;

pub fn parse(text: &str) -> anyhow::Result<EpochMs> {
    let trimmed = text.trim();
    if !trimmed.is_empty() && trimmed.chars().all(|character| character.is_ascii_digit()) {
        return Ok(EpochMs::from_millis(trimmed.parse().context("epoch milliseconds out of range")?));
    }
    let timestamp = if trimmed.len() == 10 { format!("{trimmed}T00:00:00Z") } else { trimmed.to_owned() };
    match humantime::parse_rfc3339_weak(&timestamp) {
        Ok(time) => Ok(epoch_ms(time)),
        Err(_) => bail!("`{text}` is not epoch milliseconds, a date (2026-10-20) or an RFC 3339 time"),
    }
}

pub fn now() -> EpochMs {
    epoch_ms(SystemTime::now())
}

fn epoch_ms(time: SystemTime) -> EpochMs {
    let since_epoch = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    EpochMs::from_millis(i64::try_from(since_epoch.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn accepts_millis_dates_and_rfc3339() {
        assert_eq!(parse("1760913600000").unwrap().as_millis(), 1_760_913_600_000);
        assert_eq!(parse("2026-10-20").unwrap().as_millis(), 1_792_454_400_000);
        assert_eq!(parse("2026-10-19T22:40:00Z").unwrap().as_millis(), 1_792_449_600_000);
        assert!(parse("next tuesday").is_err());
    }
}
