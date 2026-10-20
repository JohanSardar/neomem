use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;
use std::thread;

use anyhow::Context;
use neomem_engine::Engine;
use neomem_proto::{FrameError, IncomingEnvelope, read_frame, write_frame};

use crate::dispatch::{dispatch, finish_if_shutting_down};

pub fn serve(engine: &Arc<Engine>, socket_path: &Path) -> anyhow::Result<()> {
    let listener = bind(socket_path)?;
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                let engine = Arc::clone(engine);
                thread::spawn(move || serve_connection(&engine, stream));
            }
            Err(error) => tracing::warn!("accepting a socket connection failed: {error}"),
        }
    }
    Ok(())
}

fn bind(socket_path: &Path) -> anyhow::Result<UnixListener> {
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    match fs::remove_file(socket_path) {
        Ok(()) => tracing::info!("replaced a stale socket at {}", socket_path.display()),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("clearing the stale socket"),
    }
    let listener =
        UnixListener::bind(socket_path).with_context(|| format!("binding {}", socket_path.display()))?;
    fs::set_permissions(socket_path, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

fn serve_connection(engine: &Engine, mut stream: UnixStream) {
    loop {
        let incoming: IncomingEnvelope = match read_frame(&mut stream) {
            Ok(incoming) => incoming,
            Err(FrameError::Io(error)) if error.kind() == ErrorKind::UnexpectedEof => return,
            Err(error) => {
                tracing::warn!("dropping a socket connection: {error}");
                return;
            }
        };
        let response = dispatch(engine, incoming);
        if let Err(error) = write_frame(&mut stream, &response) {
            tracing::warn!("replying on the socket failed: {error}");
            return;
        }
        finish_if_shutting_down(&response);
    }
}
