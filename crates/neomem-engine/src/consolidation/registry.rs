use neomem_core::{EntityId, FactId, IntakeId};

#[derive(Debug, Default)]
pub(crate) struct Registry {
    facts: Vec<FactId>,
    entities: Vec<EntityId>,
    records: Vec<IntakeId>,
}

fn number_of<T: PartialEq + Copy>(list: &mut Vec<T>, id: T) -> usize {
    if let Some(index) = list.iter().position(|known| *known == id) {
        return index + 1;
    }
    list.push(id);
    list.len()
}

fn at<T: Copy>(list: &[T], number: i64) -> Option<T> {
    usize::try_from(number).ok()?.checked_sub(1).and_then(|index| list.get(index).copied())
}

impl Registry {
    pub(crate) fn fact_number(&mut self, id: FactId) -> usize {
        number_of(&mut self.facts, id)
    }

    pub(crate) fn entity_number(&mut self, id: EntityId) -> usize {
        number_of(&mut self.entities, id)
    }

    pub(crate) fn record_number(&mut self, id: IntakeId) -> usize {
        number_of(&mut self.records, id)
    }

    pub(crate) fn fact(&self, number: i64) -> Option<FactId> {
        at(&self.facts, number)
    }

    pub(crate) fn entity(&self, number: i64) -> Option<EntityId> {
        at(&self.entities, number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neomem_core::EpochMs;

    #[test]
    fn numbers_are_stable_and_unknown_numbers_resolve_to_nothing() {
        let mut registry = Registry::default();
        let first = FactId::from_parts(EpochMs::from_millis(1), 1);
        let second = FactId::from_parts(EpochMs::from_millis(1), 2);
        assert_eq!(registry.fact_number(first), 1);
        assert_eq!(registry.fact_number(second), 2);
        assert_eq!(registry.fact_number(first), 1);
        assert_eq!(registry.fact(2), Some(second));
        assert_eq!(registry.fact(0), None);
        assert_eq!(registry.fact(17), None);
        assert_eq!(registry.fact(-1), None);
    }
}
