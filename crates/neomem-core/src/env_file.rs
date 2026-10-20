use std::collections::BTreeMap;

pub fn env_file_entries(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.strip_prefix("export ").unwrap_or(line).split_once('='))
        .map(|(name, value)| (name.trim().to_owned(), value.trim().trim_matches('"').to_owned()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::env_file_entries;

    #[test]
    fn reads_plain_exported_and_quoted_lines_and_skips_comments() {
        let entries = env_file_entries("# keys\nONE=alpha\nexport TWO=\"beta\"\n\nnot a line\n");
        assert_eq!(entries.get("ONE").map(String::as_str), Some("alpha"));
        assert_eq!(entries.get("TWO").map(String::as_str), Some("beta"));
        assert_eq!(entries.len(), 2);
    }
}
