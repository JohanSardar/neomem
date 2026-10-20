mod client;
mod envelope;
mod frame;
mod home;
mod request;
mod response;
pub mod spool;

pub use client::{CallError, Client};
pub use envelope::{Envelope, IncomingEnvelope, PROTOCOL_VERSION};
pub use frame::{FrameError, read_frame, write_frame};
pub use home::Home;
pub use request::{RawQuery, Request};
pub use response::{
    AddReceipt, CorrectionReceipt, DriftReport, EntityView, Failure, FailureKind, IndexReport, LinkedEntity,
    ModelTestReport, RawHit, RebuildReport, ResolvedEntity, Response, ShownMemory, SourceView, StatusReport,
};
