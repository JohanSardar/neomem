use neomem_core::{EpochMs, LoggedIntake, Source};
use rusqlite::params;

use crate::intake::logged_from;
use crate::{StateDb, StateError};

const LONGEST_PAGE: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSearch<'query> {
    pub text: &'query str,
    pub from: Option<EpochMs>,
    pub to: Option<EpochMs>,
    pub source: Option<&'query Source>,
    pub limit: usize,
}

const MATCHING: &str =
    "SELECT i.id, i.recorded_at, i.source, i.occurred_start, i.occurred_end, i.tags, i.content,
        i.entity_hints, i.source_ref, i.session,
        EXISTS (SELECT 1 FROM corrections c WHERE c.intake_id = i.id)
    FROM intake_text JOIN intake i ON i.rowid = intake_text.rowid
    WHERE intake_text MATCH ?1
      AND (?2 IS NULL OR i.occurred_end >= ?2)
      AND (?3 IS NULL OR i.occurred_start <= ?3)
      AND (?4 IS NULL OR i.source = ?4)
    ORDER BY bm25(intake_text), i.occurred_start DESC
    LIMIT ?5";

const IN_WINDOW: &str = "SELECT id, recorded_at, source, occurred_start, occurred_end, tags, content,
        entity_hints, source_ref, session,
        EXISTS (SELECT 1 FROM corrections c WHERE c.intake_id = intake.id)
    FROM intake
    WHERE (?1 IS NULL OR occurred_end >= ?1)
      AND (?2 IS NULL OR occurred_start <= ?2)
      AND (?3 IS NULL OR source = ?3)
    ORDER BY occurred_start DESC
    LIMIT ?4";

impl StateDb {
    pub fn raw_search(&self, search: &RawSearch<'_>) -> Result<Vec<RawRecord>, StateError> {
        let limit = i64::try_from(search.limit.clamp(1, LONGEST_PAGE)).unwrap_or(20);
        let from = search.from.map(EpochMs::as_millis);
        let to = search.to.map(EpochMs::as_millis);
        let source = search.source.map(Source::as_str);
        let expression = match_expression(search.text);
        self.read(|connection| {
            let mut statement =
                connection.prepare_cached(if expression.is_some() { MATCHING } else { IN_WINDOW })?;
            let rows: Vec<Result<RawRecord, StateError>> = if let Some(expression) = &expression {
                statement
                    .query_map(params![expression, from, to, source, limit], |row| Ok(raw_record_from(row)))?
                    .collect::<Result<_, _>>()?
            } else {
                statement
                    .query_map(params![from, to, source, limit], |row| Ok(raw_record_from(row)))?
                    .collect::<Result<_, _>>()?
            };
            rows.into_iter().collect()
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRecord {
    pub logged: LoggedIntake,
    pub corrected: bool,
}

fn raw_record_from(row: &rusqlite::Row<'_>) -> Result<RawRecord, StateError> {
    Ok(RawRecord { logged: logged_from(row)?, corrected: row.get(10)? })
}

fn match_expression(text: &str) -> Option<String> {
    let terms: Vec<String> = text
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| format!("\"{}\"", word.to_lowercase()))
        .collect();
    if terms.is_empty() { None } else { Some(terms.join(" ")) }
}

#[cfg(test)]
mod tests {
    use super::match_expression;

    #[test]
    fn user_text_becomes_quoted_terms_so_fts_syntax_cannot_leak_in() {
        assert_eq!(match_expression("signing error"), Some("\"signing\" \"error\"".to_owned()));
        assert_eq!(
            match_expression("NEAR(a b) OR \"x\"*"),
            Some("\"near\" \"a\" \"b\" \"or\" \"x\"".to_owned())
        );
        assert_eq!(match_expression("  ...  "), None);
    }
}
