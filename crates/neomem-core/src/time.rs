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
}

#[cfg(test)]
mod tests {
    use super::EpochMs;

    #[test]
    fn serializes_as_a_bare_number() {
        let json = serde_json::to_string(&EpochMs::from_millis(1_760_796_300_000)).unwrap();
        assert_eq!(json, "1760796300000");
    }
}
