use std::io::{BufRead, ErrorKind};
use std::mem;
use std::time::Duration;

use neomem_core::ModelError;

use crate::http;

#[derive(Debug, Default)]
pub struct ServerEvent {
    pub name: String,
    pub data: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    More,
    Done,
}

pub fn read_events(
    reader: impl BufRead,
    timeout: Duration,
    mut on_event: impl FnMut(ServerEvent) -> Result<Flow, ModelError>,
) -> Result<(), ModelError> {
    let mut pending = ServerEvent::default();
    for line in reader.lines() {
        let line = line.map_err(|error| match error.kind() {
            ErrorKind::TimedOut => http::timed_out(timeout),
            _ => ModelError::Unavailable(format!("the stream broke: {error}")),
        })?;
        if line.is_empty() {
            if !pending.data.is_empty() && on_event(mem::take(&mut pending))? == Flow::Done {
                return Ok(());
            }
            pending = ServerEvent::default();
            continue;
        }
        let (field, value) = line.split_once(':').unwrap_or((line.as_str(), ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => value.clone_into(&mut pending.name),
            "data" => {
                if !pending.data.is_empty() {
                    pending.data.push('\n');
                }
                pending.data.push_str(value);
            }
            _ => {}
        }
    }
    if !pending.data.is_empty() {
        on_event(pending)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_split_on_blank_lines_and_join_multi_line_data() {
        let stream = ": comment\nevent: delta\ndata: one\ndata: two\n\ndata: [DONE]\n\n";
        let mut seen = Vec::new();
        read_events(stream.as_bytes(), Duration::from_secs(1), |event| {
            seen.push((event.name, event.data));
            Ok(Flow::More)
        })
        .unwrap();
        assert_eq!(seen, [("delta".to_owned(), "one\ntwo".to_owned()), (String::new(), "[DONE]".to_owned())]);
    }
}
