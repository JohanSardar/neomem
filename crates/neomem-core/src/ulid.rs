use std::fmt;
use std::str::FromStr;

use crate::EpochMs;

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const ENCODED_LENGTH: usize = 26;
const RANDOM_BITS: u32 = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ulid(u128);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not a 26-character Crockford base32 ULID")]
pub struct InvalidUlid(pub String);

impl Ulid {
    pub fn from_parts(timestamp: EpochMs, randomness: u128) -> Self {
        let millis = u128::try_from(timestamp.as_millis().max(0)).unwrap_or(0) & ((1 << 48) - 1);
        Self((millis << RANDOM_BITS) | (randomness & ((1 << RANDOM_BITS) - 1)))
    }

    pub fn timestamp(self) -> EpochMs {
        EpochMs::from_millis(i64::try_from(self.0 >> RANDOM_BITS).unwrap_or(i64::MAX))
    }

    pub const fn to_u128(self) -> u128 {
        self.0
    }

    pub const fn from_u128(value: u128) -> Self {
        Self(value)
    }
}

impl fmt::Display for Ulid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut encoded = [0_u8; ENCODED_LENGTH];
        let mut remaining = self.0;
        for slot in encoded.iter_mut().rev() {
            *slot = CROCKFORD[(remaining & 0x1f) as usize];
            remaining >>= 5;
        }
        formatter.write_str(std::str::from_utf8(&encoded).map_err(|_| fmt::Error)?)
    }
}

impl FromStr for Ulid {
    type Err = InvalidUlid;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.len() != ENCODED_LENGTH || !text.starts_with(|first: char| first <= '7') {
            return Err(InvalidUlid(text.to_owned()));
        }
        text.bytes()
            .try_fold(0_u128, |value, byte| {
                let digit = CROCKFORD
                    .iter()
                    .position(|candidate| *candidate == byte.to_ascii_uppercase())
                    .ok_or_else(|| InvalidUlid(text.to_owned()))?;
                Ok((value << 5) | digit as u128)
            })
            .map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_parses_back() {
        let ulid = Ulid::from_parts(EpochMs::from_millis(1_760_796_300_000), 0xDEAD_BEEF_1234);
        let text = ulid.to_string();
        assert_eq!(text.len(), 26);
        assert_eq!(text.parse::<Ulid>().unwrap(), ulid);
        assert_eq!(ulid.timestamp(), EpochMs::from_millis(1_760_796_300_000));
    }

    #[test]
    fn later_timestamps_sort_later_as_text() {
        let earlier = Ulid::from_parts(EpochMs::from_millis(1_000), u128::MAX).to_string();
        let later = Ulid::from_parts(EpochMs::from_millis(1_001), 0).to_string();
        assert!(earlier < later);
    }

    #[test]
    fn rejects_wrong_lengths_and_letters() {
        assert!("01JB".parse::<Ulid>().is_err());
        assert!("01JBUUUUUUUUUUUUUUUUUUUUUU".parse::<Ulid>().is_err());
    }
}
