use std::io::Read;

use anyhow::Context;
use neomem_core::{IntakeRecord, Source, Span, Tag};

use neomem_proto::RawQuery;

use crate::args::{AddArgs, RawArgs};
use crate::time_argument;

pub fn from_arguments(arguments: &AddArgs) -> anyhow::Result<IntakeRecord> {
    let start = arguments.start.as_deref().map(time_argument::parse).transpose()?;
    let end = arguments.end.as_deref().map(time_argument::parse).transpose()?;
    let occurred = match (start, end) {
        (Some(start), Some(end)) => Span::new(start, end)?,
        (Some(only), None) | (None, Some(only)) => Span::instant(only),
        (None, None) => Span::instant(time_argument::now()),
    };
    let tags = arguments.tags.iter().map(|tag| Tag::new(tag)).collect::<Result<Vec<_>, _>>()?;
    Ok(IntakeRecord {
        source: Source::new(&arguments.source)?,
        occurred,
        tags,
        content: content(&arguments.content)?,
        entity_hints: arguments.entities.clone(),
        source_ref: arguments.source_ref.clone(),
        session: arguments.session.clone(),
    })
}

fn content(argument: &str) -> anyhow::Result<String> {
    if argument != "-" {
        return Ok(argument.to_owned());
    }
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text).context("reading content from stdin")?;
    Ok(text)
}

pub fn raw_query(arguments: &RawArgs) -> anyhow::Result<RawQuery> {
    Ok(RawQuery {
        text: arguments.text.clone(),
        from: arguments.from.as_deref().map(time_argument::parse).transpose()?,
        to: arguments.to.as_deref().map(time_argument::parse).transpose()?,
        source: arguments.source.as_deref().map(Source::new).transpose()?,
        limit: arguments.limit,
    })
}
