use jiff::civil::Time;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use neomem_core::EpochMs;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NightlyAt {
    pub hour: i8,
    pub minute: i8,
}

impl NightlyAt {
    pub const DEFAULT: Self = Self { hour: 3, minute: 0 };

    pub fn parse(text: &str) -> Option<Self> {
        let (hour, minute) = text.trim().split_once(':')?;
        let at = Self { hour: hour.parse().ok()?, minute: minute.parse().ok()? };
        ((0..24).contains(&at.hour) && (0..60).contains(&at.minute)).then_some(at)
    }

    pub(crate) fn next_after(self, now: EpochMs) -> EpochMs {
        let fallback = now.plus_millis(86_400_000);
        let Ok(moment) = Timestamp::from_millisecond(now.as_millis()) else { return fallback };
        let zoned = moment.to_zoned(TimeZone::system());
        let Ok(time) = Time::new(self.hour, self.minute, 0, 0) else { return fallback };
        let Ok(today) = zoned.date().to_datetime(time).to_zoned(zoned.time_zone().clone()) else {
            return fallback;
        };
        let next = if today.timestamp() > moment { Ok(today) } else { today.checked_add(1.day()) };
        next.map_or(fallback, |next| EpochMs::from_millis(next.timestamp().as_millisecond()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_next_run_is_always_later_and_within_a_day() {
        let now = EpochMs::from_millis(1_792_540_800_000);
        let next = NightlyAt::DEFAULT.next_after(now);
        assert!(next > now);
        assert!(next.millis_since(now) <= 86_400_000 + 3_600_000);
        let after_that = NightlyAt::DEFAULT.next_after(next);
        assert!(after_that.millis_since(next) >= 82_800_000);
    }

    #[test]
    fn reads_hours_and_minutes() {
        assert_eq!(NightlyAt::parse("03:00"), Some(NightlyAt::DEFAULT));
        assert_eq!(NightlyAt::parse("25:00"), None);
    }
}
