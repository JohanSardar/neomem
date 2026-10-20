use neomem_core::{IntakeRecord, LoggedIntake, redact_secrets};
use neomem_proto::{AddReceipt, FailureKind, Response};
use neomem_state::{BatchKey, ExtractionJob};

use crate::Engine;
use crate::reply::failed;
use crate::worker_config::millis;

impl Engine {
    pub(crate) fn add(&self, mut record: IntakeRecord) -> Response {
        if let Err(invalid) = record.validate() {
            return failed(FailureKind::BadRequest, invalid.to_string());
        }
        record.content = redact_secrets(&record.content);
        let recorded_at = self.clock.now();
        let logged = LoggedIntake { id: self.ids.intake(recorded_at), recorded_at, record };
        let waits = if logged.record.session.is_some() {
            self.worker.session_silence
        } else {
            self.worker.batch_every
        };
        let job = ExtractionJob {
            batch: BatchKey::for_record(&logged),
            not_before: recorded_at.plus_millis(millis(waits)),
        };
        if let Err(error) = self.state.log_intake(&logged, &job) {
            return failed(FailureKind::Internal, format!("logging the intake record failed: {error}"));
        }
        if logged.record.session.is_none() {
            self.hasten_a_full_batch(&job.batch, recorded_at);
        }
        Response::Added(AddReceipt { id: logged.id, recorded_at })
    }

    fn hasten_a_full_batch(&self, batch: &BatchKey, now: neomem_core::EpochMs) {
        let full = self.state.pending_in_batch(batch).is_ok_and(|pending| pending >= self.worker.batch_size);
        if full && let Err(error) = self.state.hasten_batch(batch, now) {
            tracing::error!("releasing a full batch failed: {error}");
        }
    }
}
