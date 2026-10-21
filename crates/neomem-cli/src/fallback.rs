use anyhow::Context;
use neomem_proto::{Home, RawHit, Response, SearchQuery};
use neomem_state::{RawSearch, StateDb};

const DEFAULT_LIMIT: usize = 10;

pub fn keyword_search(home: &Home, query: &SearchQuery, down: &anyhow::Error) -> anyhow::Result<Response> {
    let state = StateDb::open_read_only(&home.state_db_path())
        .with_context(|| format!("neomemd is not running ({down:#}) and there is no intake log to search"))?;
    let search = RawSearch {
        text: &query.text,
        from: query.from,
        to: query.to,
        source: None,
        limit: query.k.unwrap_or(DEFAULT_LIMIT),
    };
    let found = state.raw_search(&search)?;
    eprintln!(
        "neomem: neomemd is not running ({down:#}); these are keyword matches from the raw intake log, not facts"
    );
    Ok(Response::RawHits(
        found.into_iter().map(|record| RawHit::from_logged(record.logged, record.corrected)).collect(),
    ))
}
