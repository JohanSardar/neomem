use neomem_engine::Engine;
use neomem_proto::{Failure, FailureKind, IncomingEnvelope, PROTOCOL_VERSION, Response};

pub fn dispatch(engine: &Engine, incoming: IncomingEnvelope) -> Response {
    if !incoming.speaks_current_protocol() && !incoming.asks_for_shutdown() {
        return Response::VersionMismatch { daemon: PROTOCOL_VERSION, client: incoming.protocol };
    }
    match incoming.into_request() {
        Ok(request) => engine.handle(request),
        Err(error) => {
            Response::Failed(Failure::new(FailureKind::BadRequest, format!("invalid request: {error}")))
        }
    }
}

pub fn finish_if_shutting_down(response: &Response) {
    if matches!(response, Response::ShuttingDown) {
        tracing::info!("neomemd shutting down on request");
        std::process::exit(0);
    }
}
