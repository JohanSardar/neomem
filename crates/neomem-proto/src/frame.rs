use std::io::{self, Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;

const MAX_FRAME_BYTES: u32 = 64 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("the connection failed: {0}")]
    Io(#[from] io::Error),
    #[error("the frame is not valid JSON for this message: {0}")]
    Json(#[from] serde_json::Error),
    #[error("a frame of {0} bytes is over the 64 MiB limit")]
    TooLarge(u32),
}

pub fn write_frame(writer: &mut impl Write, message: &impl Serialize) -> Result<(), FrameError> {
    let body = serde_json::to_vec(message)?;
    let length = u32::try_from(body.len()).map_err(|_| FrameError::TooLarge(u32::MAX))?;
    if length > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(length));
    }
    let mut buffer = Vec::with_capacity(body.len() + 4);
    buffer.extend_from_slice(&length.to_be_bytes());
    buffer.extend_from_slice(&body);
    writer.write_all(&buffer)?;
    writer.flush()?;
    Ok(())
}

pub fn read_frame<T: DeserializeOwned>(reader: &mut impl Read) -> Result<T, FrameError> {
    let mut length_bytes = [0_u8; 4];
    reader.read_exact(&mut length_bytes)?;
    let length = u32::from_be_bytes(length_bytes);
    if length > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(length));
    }
    let mut body = vec![0_u8; length as usize];
    reader.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Envelope, Request};

    #[test]
    fn a_frame_round_trips() {
        let mut wire = Vec::new();
        write_frame(&mut wire, &Envelope::current(Request::Status)).unwrap();
        let back: Envelope = read_frame(&mut wire.as_slice()).unwrap();
        assert_eq!(back.request, Request::Status);
    }

    #[test]
    fn oversized_frames_are_refused_before_reading_the_body() {
        let wire = (MAX_FRAME_BYTES + 1).to_be_bytes();
        let outcome: Result<Envelope, _> = read_frame(&mut wire.as_slice());
        assert!(matches!(outcome, Err(FrameError::TooLarge(_))));
    }
}
