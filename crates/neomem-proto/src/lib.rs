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
pub use request::{RawQuery, Request, SearchQuery};
pub use response::{
    AddReceipt, CorrectionReceipt, DreamReport, DriftReport, EntityRef, EntityView, Failure, FailureKind,
    HistoryView, IndexReport, LinkedEntity, LinkedFact, ModelTestReport, RawHit, RebuildReport,
    ResolvedEntity, Response, SearchHit, SearchResults, SearchTrace, ShownMemory, SourceView,
    SpecialistReport, StatusReport,
};
