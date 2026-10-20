use neomem_proto::{FailureKind, RawHit, RawQuery, Response};
use neomem_state::RawSearch;

use crate::Engine;
use crate::reply::failed;

impl Engine {
    pub(crate) fn raw(&self, query: &RawQuery) -> Response {
        let search = RawSearch {
            text: &query.text,
            from: query.from,
            to: query.to,
            source: query.source.as_ref(),
            limit: query.limit,
        };
        match self.state.raw_search(&search) {
            Ok(found) => Response::RawHits(
                found
                    .into_iter()
                    .map(|logged| RawHit {
                        id: logged.id,
                        source: logged.record.source,
                        occurred: logged.record.occurred,
                        recorded_at: logged.recorded_at,
                        tags: logged.record.tags,
                        content: logged.record.content,
                        corrected: false,
                    })
                    .collect(),
            ),
            Err(error) => failed(FailureKind::Internal, format!("raw search failed: {error}")),
        }
    }
}
