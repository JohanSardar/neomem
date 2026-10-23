use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EpochMs(i64);

impl EpochMs {
    pub const fn from_millis(millis: i64) -> Self {
        Self(millis)
    }

    pub const fn as_millis(self) -> i64 {
        self.0
    }

    pub const fn millis_since(self, earlier: Self) -> i64 {
        self.0.saturating_sub(earlier.0)
    }

    pub const fn plus_millis(self, millis: i64) -> Self {
        Self(self.0.saturating_add(millis))
    }

    pub fn utc_text(self) -> String {
        let seconds = self.0.div_euclid(MILLIS_PER_SECOND);
        let (days, second_of_day) =
            (seconds.div_euclid(SECONDS_PER_DAY), seconds.rem_euclid(SECONDS_PER_DAY));
        let (year, month, day) = civil_from_days(days);
        format!(
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
            second_of_day / 3600,
            second_of_day % 3600 / 60,
            second_of_day % 60
        )
    }
}

const MILLIS_PER_SECOND: i64 = 1000;
const SECONDS_PER_DAY: i64 = 86_400;

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::EpochMs;

    #[test]
    fn reads_as_a_utc_timestamp() {
        assert_eq!(EpochMs::from_millis(1_684_540_800_000).utc_text(), "2023-05-20T00:00:00Z");
        assert_eq!(EpochMs::from_millis(1_792_713_461_370).utc_text(), "2026-10-22T23:57:41Z");
        assert_eq!(EpochMs::from_millis(951_782_400_000).utc_text(), "2000-02-29T00:00:00Z");
        assert_eq!(EpochMs::from_millis(-1).utc_text(), "1969-12-31T23:59:59Z");
    }

    #[test]
    fn serializes_as_a_bare_number() {
        let json = serde_json::to_string(&EpochMs::from_millis(1_760_796_300_000)).unwrap();
        assert_eq!(json, "1760796300000");
    }
}
