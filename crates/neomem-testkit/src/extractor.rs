use serde_json::{Value, json};

pub const EXTRACTOR_MARKER: &str = "You are neomem's memory extractor";
const LONGEST_FACT: usize = 400;

struct PromptRecord {
    number: usize,
    hints: Vec<String>,
    content: String,
}

pub fn default_extraction(prompt: &str) -> Value {
    let records = new_records(prompt);
    let known = known_entities(prompt);
    let mut entities: Vec<Value> = Vec::new();
    let mut facts = Vec::new();
    for record in records {
        let names = if record.hints.is_empty() { vec!["the user".to_owned()] } else { record.hints.clone() };
        let mut about = Vec::new();
        for name in names {
            let reference = format!("E{}", reference_for(&mut entities, &name, &known));
            about.push(reference);
        }
        let content: String = record.content.chars().take(LONGEST_FACT).collect();
        facts.push(json!({
            "content": content,
            "sources": [record.number],
            "about": about,
            "valid_from": null,
            "importance": 5,
            "durability": "ordinary",
            "tags": [],
        }));
    }
    json!({ "facts": facts, "entities": entities })
}

fn reference_for(entities: &mut Vec<Value>, name: &str, known: &[(usize, String)]) -> usize {
    if let Some(index) = entities
        .iter()
        .position(|entity| entity["name"].as_str().is_some_and(|known| known.eq_ignore_ascii_case(name)))
    {
        return index + 1;
    }
    let existing =
        known.iter().find(|(_, known_name)| known_name.eq_ignore_ascii_case(name)).map(|(number, _)| *number);
    let kind = if name == "the user" { "person" } else { "thing" };
    let number = entities.len() + 1;
    entities.push(json!({ "ref": format!("E{number}"), "name": name, "kind": kind, "aliases": [], "existing": existing }));
    number
}

fn section<'prompt>(prompt: &'prompt str, heading: &str) -> impl Iterator<Item = &'prompt str> {
    prompt
        .lines()
        .skip_while(move |line| !line.starts_with(heading))
        .skip(1)
        .take_while(|line| !line.is_empty() || line.starts_with("    "))
}

fn new_records(prompt: &str) -> Vec<PromptRecord> {
    let mut records: Vec<PromptRecord> = Vec::new();
    for line in section(prompt, "New records:") {
        if let Some(content) = line.strip_prefix("    ") {
            if let Some(record) = records.last_mut() {
                if !record.content.is_empty() {
                    record.content.push(' ');
                }
                record.content.push_str(content.trim());
            }
        } else if let Some((number, header)) = numbered(line) {
            let hints = header
                .split(" | ")
                .find_map(|part| part.strip_prefix("hints: "))
                .map(|hints| hints.split(", ").map(str::to_owned).collect())
                .unwrap_or_default();
            records.push(PromptRecord { number, hints, content: String::new() });
        }
    }
    records
}

fn known_entities(prompt: &str) -> Vec<(usize, String)> {
    section(prompt, "Known entities:")
        .filter_map(numbered)
        .map(|(number, rest)| (number, rest.split(" (").next().unwrap_or(rest).to_owned()))
        .collect()
}

fn numbered(line: &str) -> Option<(usize, &str)> {
    let rest = line.strip_prefix('[')?;
    let (number, rest) = rest.split_once("] ")?;
    Some((number.parse().ok()?, rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROMPT: &str = "Today is 2026-10-20 (Tuesday).\n\nNew records:\n[1] 2026-10-20 14:03 (Tuesday) | claude-code | session s | hints: Edge\n    The Edge beta opens Thursday\n    for 50 testers\n[2] 2026-10-20 14:05 (Tuesday) | note\n    Dentist at three\n\nKnown entities:\n[1] Edge (project; also Edge app)\n\nRelated facts (context only):\n\nKnown tags: code\n";

    #[test]
    fn one_fact_per_record_about_its_hints_or_the_user() {
        let output = default_extraction(PROMPT);
        assert_eq!(output["facts"][0]["content"], "The Edge beta opens Thursday for 50 testers");
        assert_eq!(output["facts"][0]["sources"], json!([1]));
        assert_eq!(output["entities"][0]["existing"], json!(1));
        assert_eq!(output["entities"][1]["name"], "the user");
        assert_eq!(output["facts"][1]["about"], json!(["E2"]));
    }
}
