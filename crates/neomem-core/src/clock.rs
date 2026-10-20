use std::fmt::Debug;

use crate::EpochMs;

pub trait Clock: Send + Sync + Debug {
    fn now(&self) -> EpochMs;
}
