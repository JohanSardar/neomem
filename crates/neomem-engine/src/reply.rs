use neomem_proto::{Failure, FailureKind, Response};

pub fn failed(kind: FailureKind, message: impl Into<String>) -> Response {
    Response::Failed(Failure::new(kind, message))
}
