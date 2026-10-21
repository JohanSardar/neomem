use std::fmt::Write as _;
use std::sync::Arc;

use neomem_core::{Entity, EpochMs, Fact, LlmMessage, LlmRequest, LoggedIntake, Tag};

pub const INSTRUCTIONS: &str = include_str!("../prompts/extract.md");
pub const OUTPUT_SCHEMA: &str = include_str!("../prompts/extract.schema.json");
const LONGEST_REPLY_TOKENS: u32 = 4096;

#[derive(Debug)]
pub(crate) struct ExtractionInput {
    pub(crate) records: Vec<LoggedIntake>,
    pub(crate) earlier: Vec<LoggedIntake>,
    pub(crate) entities: Vec<Arc<Entity>>,
    pub(crate) facts: Vec<Arc<Fact>>,
    pub(crate) tags: Vec<Tag>,
    pub(crate) today: EpochMs,
}

impl ExtractionInput {
    pub(crate) fn request(&self) -> LlmRequest {
        LlmRequest {
            system: INSTRUCTIONS.to_owned(),
            messages: vec![LlmMessage::User(self.message())],
            response_schema: Some(OUTPUT_SCHEMA.to_owned()),
            tools: Vec::new(),
            max_output_tokens: LONGEST_REPLY_TOKENS,
        }
    }

    fn message(&self) -> String {
        let mut text = format!("Today is {}.\n\nNew records:\n", day(self.today));
        for (number, logged) in self.records.iter().enumerate() {
            write_record(&mut text, &format!("[{}]", number + 1), logged);
        }
        if !self.earlier.is_empty() {
            text.push_str("\nEarlier turns in this session (context only):\n");
            for logged in &self.earlier {
                write_record(&mut text, "-", logged);
            }
        }
        text.push_str("\nKnown entities:\n");
        for (number, entity) in self.entities.iter().enumerate() {
            let aliases = if entity.aliases().is_empty() {
                String::new()
            } else {
                format!("; also {}", entity.aliases().join(", "))
            };
            let _ = writeln!(text, "[{}] {} ({}{aliases})", number + 1, entity.name(), entity.kind());
        }
        text.push_str("\nRelated facts (context only):\n");
        for (number, fact) in self.facts.iter().enumerate() {
            let _ = writeln!(text, "[{}] {}", number + 1, fact.content());
        }
        let tags: Vec<&str> = self.tags.iter().map(Tag::as_str).collect();
        let _ = writeln!(text, "\nKnown tags: {}", tags.join(", "));
        text
    }
}

fn write_record(text: &mut String, label: &str, logged: &LoggedIntake) {
    let record = &logged.record;
    let session =
        record.session.as_deref().map(|session| format!(" | session {session}")).unwrap_or_default();
    let hints = if record.entity_hints.is_empty() {
        String::new()
    } else {
        format!(" | hints: {}", record.entity_hints.join(", "))
    };
    let _ = writeln!(text, "{label} {} | {}{session}{hints}", moment(record.occurred.start), record.source);
    for line in record.content.lines() {
        let _ = writeln!(text, "    {line}");
    }
}

fn moment(at: EpochMs) -> String {
    jiff::Timestamp::from_millisecond(at.as_millis())
        .map(|time| time.to_zoned(jiff::tz::TimeZone::system()).strftime("%Y-%m-%d %H:%M (%A)").to_string())
        .unwrap_or_default()
}

fn day(at: EpochMs) -> String {
    jiff::Timestamp::from_millisecond(at.as_millis())
        .map(|time| time.to_zoned(jiff::tz::TimeZone::system()).strftime("%Y-%m-%d (%A)").to_string())
        .unwrap_or_default()
}
