use std::io;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use crate::{Envelope, FrameError, Home, Request, Response, read_frame, write_frame};

#[derive(Debug)]
pub struct Client {
    stream: UnixStream,
}

#[derive(Debug, thiserror::Error)]
pub enum CallError {
    #[error("the daemon is not reachable: {0}")]
    Unreachable(#[from] io::Error),
    #[error(transparent)]
    Frame(#[from] FrameError),
}

impl Client {
    pub fn connect(home: &Home, timeout: Duration) -> Result<Self, CallError> {
        let stream = UnixStream::connect(home.socket_path())?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        Ok(Self { stream })
    }

    pub fn set_timeout(&self, timeout: Duration) -> Result<(), CallError> {
        self.stream.set_read_timeout(Some(timeout))?;
        self.stream.set_write_timeout(Some(timeout))?;
        Ok(())
    }

    pub fn call(&mut self, request: Request) -> Result<Response, CallError> {
        self.send(&Envelope::current(request))
    }

    pub fn send(&mut self, envelope: &Envelope) -> Result<Response, CallError> {
        write_frame(&mut self.stream, envelope)?;
        Ok(read_frame(&mut self.stream)?)
    }
}
