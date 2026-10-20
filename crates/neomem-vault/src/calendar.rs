use neomem_core::EpochMs;

const MILLIS_PER_DAY: i64 = 86_400_000;

pub fn year_and_month(moment: EpochMs) -> (i64, u32) {
    let days = moment.as_millis().div_euclid(MILLIS_PER_DAY);
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, u32::try_from(month).unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_epoch_milliseconds_to_the_utc_year_and_month() {
        assert_eq!(year_and_month(EpochMs::from_millis(0)), (1970, 1));
        assert_eq!(year_and_month(EpochMs::from_millis(1_792_454_400_000)), (2026, 10));
        assert_eq!(year_and_month(EpochMs::from_millis(1_709_164_800_000)), (2024, 2));
        assert_eq!(year_and_month(EpochMs::from_millis(-1)), (1969, 12));
    }
}
