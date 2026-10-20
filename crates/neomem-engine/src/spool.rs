use neomem_proto::{Home, Response, spool};

use crate::Engine;

impl Engine {
    pub(crate) fn drain_spool(&self) {
        let home = Home::at(&self.home);
        for spooled in spool::spooled(&home) {
            match spooled
                .envelope
                .map(|envelope| (envelope.speaks_current_protocol(), envelope.into_request()))
            {
                Ok((true, Ok(request))) => match self.handle(request) {
                    Response::Failed(failure) if failure.kind == neomem_proto::FailureKind::Internal => {
                        tracing::warn!("a spooled request will be retried: {}", failure.message);
                    }
                    _ => finish_spooled(&spooled.path),
                },
                _ => self.set_spooled_aside(&home, &spooled.path),
            }
        }
    }

    fn set_spooled_aside(&self, home: &Home, path: &std::path::Path) {
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        tracing::warn!("spool file {name} is unreadable and was set aside");
        if let Err(error) = self.state.log_error(self.clock.now(), "spool_unreadable", &name) {
            tracing::error!("recording a spool error failed: {error}");
        }
        if let Err(error) = spool::set_aside(home, path) {
            tracing::error!("setting a spool file aside failed: {error}");
        }
    }
}

fn finish_spooled(path: &std::path::Path) {
    if let Err(error) = spool::finish(path) {
        tracing::error!("removing a drained spool file failed: {error}");
    }
}
