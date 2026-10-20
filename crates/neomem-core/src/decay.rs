use crate::{Durability, EpochMs, Importance};

const FLOOR: f32 = 0.3;
const CEILING: f32 = 1.5;
const BASE_EXPONENT: f64 = 0.5;
const PREFERENCE_EXPONENT: f64 = 0.25;
const IMPORTANCE_SLOWS_DECAY_BY: f64 = 0.5;
const MILLIS_PER_DAY: f64 = 86_400_000.0;
const YOUNGEST_AGE_DAYS: f64 = 1.0 / 1440.0;
const INVALID_PENALTY: f32 = 0.2;
const IMPORTANCE_WEIGHT: f32 = 0.5;

pub fn decay_factor(
    evidence: impl Iterator<Item = EpochMs>,
    now: EpochMs,
    durability: Durability,
    importance: Importance,
) -> f32 {
    let base = match durability {
        Durability::Stable => return 1.0,
        Durability::Preference => PREFERENCE_EXPONENT,
        Durability::Ordinary => BASE_EXPONENT,
    };
    let exponent = base * (1.0 - IMPORTANCE_SLOWS_DECAY_BY * f64::from(importance.normalized()));
    let strength: f64 = evidence
        .map(|event| {
            let age_days = millis_to_days(now.millis_since(event)).max(YOUNGEST_AGE_DAYS);
            age_days.powf(-exponent)
        })
        .sum();
    if strength <= 0.0 {
        return FLOOR;
    }
    let activation = strength.ln();
    let squashed = 1.0 / (1.0 + (-activation).exp());
    FLOOR + (CEILING - FLOOR) * narrow(squashed)
}

pub fn rank_score(relevance: f32, decay: f32, importance: Importance, invalid: bool) -> f32 {
    let penalty = if invalid { INVALID_PENALTY } else { 1.0 };
    relevance * decay * (1.0 + IMPORTANCE_WEIGHT * importance.normalized()) * penalty
}

#[allow(clippy::cast_precision_loss)]
fn millis_to_days(millis: i64) -> f64 {
    millis as f64 / MILLIS_PER_DAY
}

#[allow(clippy::cast_possible_truncation)]
fn narrow(value: f64) -> f32 {
    value as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400_000;
    const NOW: i64 = 1_800_000_000_000;

    fn ago(days: f64) -> EpochMs {
        #[allow(clippy::cast_possible_truncation)]
        EpochMs::from_millis(NOW - (days * 86_400_000.0) as i64)
    }

    fn importance(value: u8) -> Importance {
        Importance::new(value).unwrap()
    }

    fn ordinary(events: &[f64], value: u8) -> f32 {
        decay_factor(
            events.iter().map(|days| ago(*days)),
            EpochMs::from_millis(NOW),
            Durability::Ordinary,
            importance(value),
        )
    }

    #[test]
    fn stable_facts_never_decay() {
        let ancient = [3650.0];
        let stable = decay_factor(
            ancient.iter().map(|days| ago(*days)),
            EpochMs::from_millis(NOW),
            Durability::Stable,
            importance(1),
        );
        assert!((stable - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn fresh_facts_rank_near_the_ceiling_and_old_ones_stay_above_the_floor() {
        assert!(ordinary(&[0.0], 1) > 1.4);
        let one_day = ordinary(&[1.0], 1);
        assert!((one_day - 0.9).abs() < 0.01, "{one_day}");
        let five_years = ordinary(&[1825.0], 1);
        assert!((FLOOR..0.45).contains(&five_years), "{five_years}");
    }

    #[test]
    fn more_evidence_preference_and_importance_all_slow_the_fall() {
        assert!(ordinary(&[30.0, 20.0, 10.0], 1) > ordinary(&[30.0], 1));
        let preference = decay_factor(
            std::iter::once(ago(90.0)),
            EpochMs::from_millis(NOW),
            Durability::Preference,
            importance(1),
        );
        assert!(preference > ordinary(&[90.0], 1));
        assert!(ordinary(&[90.0], 10) > ordinary(&[90.0], 1));
        assert!(ordinary(&[90.0], 10) > 0.0 && DAY > 0);
    }

    #[test]
    fn invalid_facts_rank_at_a_fifth() {
        let valid = rank_score(0.8, 1.0, importance(1), false);
        let invalid = rank_score(0.8, 1.0, importance(1), true);
        assert!((invalid - valid * 0.2).abs() < 1e-6);
        assert!(rank_score(0.8, 1.0, importance(10), false) > valid);
    }
}
