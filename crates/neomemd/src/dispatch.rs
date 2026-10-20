use std::sync::Arc;

use neomem_engine::Engine;
use neomem_proto::{Failure, FailureKind, IncomingEnvelope, PROTOCOL_VERSION, Request, Response};

use crate::daemon_clock::DaemonClock;

#[derive(Debug)]
pub struct Dispatcher {
    engine: Arc<Engine>,
    clock: Arc<DaemonClock>,
}

impl Dispatcher {
    pub fn new(engine: Arc<Engine>, clock: Arc<DaemonClock>) -> Self {
        Self { engine, clock }
    }

    pub fn engine(&self) -> &Arc<Engine> {
        &self.engine
    }

    pub fn dispatch(&self, incoming: IncomingEnvelope) -> Response {
        if !incoming.speaks_current_protocol() && !incoming.asks_for_shutdown() {
            return Response::VersionMismatch { daemon: PROTOCOL_VERSION, client: incoming.protocol };
        }
        match incoming.into_request() {
            Ok(Request::SetClock { now }) if self.clock.set(now) => Response::ClockSet { now },
            Ok(request) => self.engine.handle(request),
            Err(error) => {
                Response::Failed(Failure::new(FailureKind::BadRequest, format!("invalid request: {error}")))
            }
        }
    }
}

pub fn finish_if_shutting_down(response: &Response) {
    if matches!(response, Response::ShuttingDown) {
        tracing::info!("neomemd shutting down on request");
        std::process::exit(0);
    }
}
