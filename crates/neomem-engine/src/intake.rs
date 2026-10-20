use neomem_core::{IntakeRecord, LoggedIntake, redact_secrets};
use neomem_proto::{AddReceipt, FailureKind, Response};
use neomem_state::{BatchKey, ExtractionJob};

use crate::Engine;
use crate::reply::failed;

impl Engine {
    pub(crate) fn add(&self, mut record: IntakeRecord) -> Response {
        if let Err(invalid) = record.validate() {
            return failed(FailureKind::BadRequest, invalid.to_string());
        }
        record.content = redact_secrets(&record.content);
        let recorded_at = self.clock.now();
        let logged = LoggedIntake { id: self.ids.intake(recorded_at), recorded_at, record };
        let job = ExtractionJob { batch: BatchKey::for_record(&logged), not_before: recorded_at };
        match self.state.log_intake(&logged, &job) {
            Ok(()) => Response::Added(AddReceipt { id: logged.id, recorded_at }),
            Err(error) => failed(FailureKind::Internal, format!("logging the intake record failed: {error}")),
        }
    }
}
