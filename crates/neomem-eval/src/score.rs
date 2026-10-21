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
