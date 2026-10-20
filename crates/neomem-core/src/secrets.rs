const REDACTED: &str = "[redacted]";

const TOKEN_PREFIXES: &[(&str, usize)] = &[
    ("sk-", 20),
    ("sk_live_", 16),
    ("sk_test_", 16),
    ("rk_live_", 16),
    ("ghp_", 20),
    ("gho_", 20),
    ("ghs_", 20),
    ("ghu_", 20),
    ("github_pat_", 20),
    ("glpat-", 16),
    ("xoxb-", 16),
    ("xoxp-", 16),
    ("xapp-", 16),
    ("hf_", 20),
    ("AIza", 30),
    ("AKIA", 16),
    ("ya29.", 20),
    ("npm_", 20),
];

const SECRET_NAMES: &[&str] = &["key", "token", "secret", "password", "passwd", "pwd", "credential", "auth"];

pub fn redact_secrets(text: &str) -> String {
    let without_keys = redact_private_key_blocks(text);
    let mut redacted = String::with_capacity(without_keys.len());
    let mut word = String::new();
    for character in without_keys.chars() {
        if is_word_character(character) {
            word.push(character);
        } else {
            redacted.push_str(&redact_word(&word));
            word.clear();
            redacted.push(character);
        }
    }
    redacted.push_str(&redact_word(&word));
    redacted
}

fn is_word_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || "_-.=+/:".contains(character)
}

fn redact_word(word: &str) -> String {
    if let Some((name, value)) = word.split_once('=')
        && names_a_secret(name)
        && value.len() >= 8
    {
        return format!("{name}={REDACTED}");
    }
    if let Some((name, value)) = word.split_once(':')
        && names_a_secret(name)
        && value.len() >= 12
        && !value.starts_with("//")
    {
        return format!("{name}:{REDACTED}");
    }
    if looks_like_token(word) || looks_like_jwt(word) {
        return REDACTED.to_owned();
    }
    word.to_owned()
}

fn names_a_secret(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    SECRET_NAMES.iter().any(|secret| lowered.ends_with(secret) || lowered.contains(&format!("{secret}_")))
}

fn looks_like_token(word: &str) -> bool {
    let trimmed = word.trim_end_matches(['.', ':']);
    TOKEN_PREFIXES.iter().any(|(prefix, minimum_tail)| {
        trimmed.strip_prefix(prefix).is_some_and(|tail| {
            tail.len() >= *minimum_tail
                && tail.chars().all(|character| character.is_ascii_alphanumeric() || "_-".contains(character))
        })
    })
}

fn looks_like_jwt(word: &str) -> bool {
    let parts: Vec<&str> = word.split('.').collect();
    parts.len() == 3 && parts[0].starts_with("eyJ") && parts.iter().all(|part| part.len() >= 10)
}

fn redact_private_key_blocks(text: &str) -> String {
    let mut remaining = text;
    let mut result = String::with_capacity(text.len());
    while let Some(start) = remaining.find("-----BEGIN ") {
        let after_start = &remaining[start..];
        let Some(header_end) = after_start.find("PRIVATE KEY-----") else { break };
        result.push_str(&remaining[..start]);
        let block = &after_start[header_end..];
        let block_end = block
            .find("-----END ")
            .and_then(|end| block[end..].find("KEY-----").map(|close| end + close + 8));
        result.push_str(REDACTED);
        remaining = block_end.map_or("", |close| &block[close..]);
    }
    result.push_str(remaining);
    result
}

#[cfg(test)]
mod tests {
    use super::redact_secrets;

    const FAKE_TAIL: &str = "Zq81xYvB2nT7kLm3Pq9RsUv4WxYz0AbC";

    fn fake(prefix: &str) -> String {
        [prefix, FAKE_TAIL].concat()
    }

    #[test]
    fn provider_keys_are_removed() {
        let text = format!("export OPENAI_API_KEY={} and {}", fake("sk-proj-"), fake("ghp_"));
        let redacted = redact_secrets(&text);
        assert!(!redacted.contains(FAKE_TAIL));
        assert!(redacted.starts_with("export OPENAI_API_KEY=[redacted]"));
    }

    #[test]
    fn named_secret_assignments_lose_their_values() {
        assert_eq!(redact_secrets("password=hunter2hunter2 ok"), "password=[redacted] ok");
        assert_eq!(redact_secrets("db_token=abcdefgh1234"), "db_token=[redacted]");
    }

    #[test]
    fn private_key_blocks_and_jwts_are_removed() {
        let begin = ["-----BEGIN RSA PRIVATE", " KEY-----"].concat();
        let text = format!("key:\n{begin}\nMIIEow\n-----END RSA PRIVATE KEY-----\ndone");
        assert_eq!(redact_secrets(&text), "key:\n[redacted]\ndone");
        let jwt = format!("Bearer {}.{}.{}", fake("eyJ"), fake("eyJ"), FAKE_TAIL);
        assert_eq!(redact_secrets(&jwt), "Bearer [redacted]");
    }

    #[test]
    fn ordinary_prose_and_urls_are_untouched() {
        let text = "Edge ships Nov 1. See https://example.com/docs:intro and the sk-learn notes; keyboard=fine";
        assert_eq!(redact_secrets(text), text);
    }
}
