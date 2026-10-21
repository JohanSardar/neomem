use neomem_proto::SearchResults;
use std::collections::BTreeMap;
pub fn retrieval(
    results: Option<&SearchResults>,
    evidence: &BTreeMap<String, String>,
    sessions: &[String],
) -> (Option<f64>, Option<f64>, Option<f64>) {
    let retrieval = results.filter(|_| !sessions.is_empty());
    let recall = |k| {
        retrieval.map(|r| {
            let covered: std::collections::BTreeSet<_> = r
                .hits
                .iter()
                .take(k)
                .flat_map(|h| h.fact.sources().iter().map(ToString::to_string))
                .filter_map(|s| evidence.get(&s).cloned())
                .collect();
            ratio(covered.len(), sessions.len())
        })
    };
    let precision5 = retrieval.map(|r| {
        ratio(
            r.hits
                .iter()
                .take(5)
                .filter(|h| h.fact.sources().iter().any(|s| evidence.contains_key(&s.to_string())))
                .count(),
            5,
        )
    });
    (recall(5), recall(10), precision5)
}
fn ratio(numerator: usize, denominator: usize) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    {
        numerator as f64 / denominator.max(1) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recall_counts_unique_evidence_sessions_and_precision_counts_slots() {
        let fact = json!({"id":"fct_01M5EBP30P6YSP08TFH9DN6Y22","about":["ent_01M5EBP30EESFSS99RBRPQ0DV8"],"content":"The bicycle is blue.","durability":"ordinary","evidence":[],"importance":5,"invalid_at":null,"links":[],"recorded_at":1_792_557_124_630_i64,"sources":["int_01M5EBP1X69HAZ0HM61Q312SJ3"],"tags":[],"updated_at":1_792_557_124_630_i64,"valid_at":1_780_401_600_000_i64});
        let hit = json!({"fact":fact,"score":1.0,"similarity":1.0,"decay":1.0,"replaced":[],"conflict":false,"linked":[],"entities":[]});
        let results: SearchResults =
            serde_json::from_value(json!({"hits":[hit.clone(),hit],"model":"oracle","trace":null})).unwrap();
        let evidence = BTreeMap::from([("int_01M5EBP1X69HAZ0HM61Q312SJ3".to_owned(), "s1".to_owned())]);
        let sessions = vec!["s1".to_owned(), "s2".to_owned()];
        assert_eq!(retrieval(Some(&results), &evidence, &sessions), (Some(0.5), Some(0.5), Some(0.4)));
        assert_eq!(retrieval(Some(&results), &BTreeMap::new(), &sessions), (Some(0.0), Some(0.0), Some(0.0)));
        assert_eq!(retrieval(Some(&results), &evidence, &[]), (None, None, None));
        assert_eq!(retrieval(None, &evidence, &sessions), (None, None, None));
    }
}
