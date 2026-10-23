mod client;
mod envelope;
mod files;
mod frame;
mod home;
mod request;
mod response;
mod shown;
pub mod spool;

pub use client::{CallError, Client};
pub use envelope::{Envelope, IncomingEnvelope, PROTOCOL_VERSION};
pub use files::publish_file;
pub use frame::{FrameError, read_frame, write_frame};
pub use home::Home;
pub use request::{RawQuery, RecallQuery, RecallTurn, Request, SearchQuery};
pub use response::{
    AddReceipt, CorrectionReceipt, DreamReport, DriftReport, EntityRef, EntityView, Failure, FailureKind,
    HistoryView, IndexReport, InstallOutcome, InstallReport, InstallStep, LinkedEntity, LinkedFact,
    ModelTestReport, RawHit, RebuildReport, RecallMemory, ResolvedEntity, Response, SearchHit, SearchResults,
    SearchTrace, ShownMemory, SourceView, SpecialistReport, StatusReport,
};

mod schema;
pub use schema::contract_schema;
pub use shown::shown_memory;
