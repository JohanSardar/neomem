use neomem_core::IntakeId;
use neomem_proto::{CorrectionReceipt, FailureKind, RawHit, RawQuery, Response};
use neomem_state::{CorrectionOutcome, RawSearch};

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
                    .map(|record| RawHit::from_logged(record.logged, record.corrected))
                    .collect(),
            ),
            Err(error) => failed(FailureKind::Internal, format!("raw search failed: {error}")),
        }
    }
}

impl Engine {
    pub(crate) fn correct(&self, intake: IntakeId, reason: &str) -> Response {
        let recorded_at = self.clock.now();
        match self.state.correct(intake, reason, recorded_at) {
            Ok(CorrectionOutcome::Recorded) => Response::Corrected(CorrectionReceipt { intake, recorded_at }),
            Ok(CorrectionOutcome::UnknownIntake) => {
                failed(FailureKind::NotFound, format!("there is no intake record {intake}"))
            }
            Err(error) => failed(FailureKind::Internal, format!("recording the correction failed: {error}")),
        }
    }
}
