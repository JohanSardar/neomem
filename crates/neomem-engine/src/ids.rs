use std::hash::{BuildHasher, RandomState};
use std::sync::Mutex;

use neomem_core::{EpochMs, IntakeId};

#[derive(Debug)]
pub struct IdMint {
    entropy: RandomState,
    last: Mutex<MintedLast>,
}

#[derive(Debug, Default)]
struct MintedLast {
    millis: i64,
    randomness: u128,
    draws: u64,
}

const RANDOM_MASK: u128 = (1 << 80) - 1;

impl IdMint {
    pub fn new() -> Self {
        Self { entropy: RandomState::new(), last: Mutex::new(MintedLast::default()) }
    }

    pub fn intake(&self, now: EpochMs) -> IntakeId {
        let (millis, randomness) = self.next(now);
        IntakeId::from_parts(millis, randomness)
    }

    fn next(&self, now: EpochMs) -> (EpochMs, u128) {
        let mut last = self.last.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        last.draws += 1;
        if now.as_millis() > last.millis {
            last.millis = now.as_millis();
            last.randomness = self.fresh_randomness(last.draws);
        } else {
            last.randomness = (last.randomness + 1) & RANDOM_MASK;
        }
        (EpochMs::from_millis(last.millis), last.randomness)
    }

    fn fresh_randomness(&self, draw: u64) -> u128 {
        let high = u128::from(self.entropy.hash_one((draw, 0_u8)));
        let low = u128::from(self.entropy.hash_one((draw, 1_u8)));
        ((high << 64) | low) & (RANDOM_MASK >> 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_minted_in_the_same_millisecond_stay_unique_and_ordered() {
        let mint = IdMint::new();
        let now = EpochMs::from_millis(1_760_000_000_000);
        let ids: Vec<IntakeId> = (0..1000).map(|_| mint.intake(now)).collect();
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn a_clock_that_steps_back_never_produces_an_older_id() {
        let mint = IdMint::new();
        let later = mint.intake(EpochMs::from_millis(2_000));
        let after_step_back = mint.intake(EpochMs::from_millis(1_000));
        assert!(after_step_back > later);
    }
}
