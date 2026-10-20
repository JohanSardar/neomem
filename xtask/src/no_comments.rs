use std::fs;
use std::path::Path;

use anyhow::Context;
use ra_ap_rustc_lexer::{FrontmatterAllowed, TokenKind, tokenize};

use crate::violation::Violation;
use crate::workspace;

pub fn check(root: &Path) -> anyhow::Result<Vec<Violation>> {
    let mut violations = Vec::new();
    for file in workspace::rust_sources(root) {
        let source = fs::read_to_string(&file)
            .with_context(|| format!("reading {}", file.display()))?;
        let relative = file.strip_prefix(root).unwrap_or(&file).to_path_buf();
        violations.extend(comment_lines(&source).into_iter().map(|line| Violation {
            check: "no-comments",
            file: relative.clone(),
            line,
            message: "comments are banned; let names and types explain the code".to_owned(),
        }));
    }
    Ok(violations)
}

fn comment_lines(source: &str) -> Vec<usize> {
    let mut offset = 0;
    let mut lines = Vec::new();
    for token in tokenize(source, FrontmatterAllowed::No) {
        let length = token.len as usize;
        if is_comment(token.kind) {
            lines.push(line_number_at(source, offset));
        }
        offset += length;
    }
    lines
}

fn is_comment(kind: TokenKind) -> bool {
    matches!(kind, TokenKind::LineComment { .. } | TokenKind::BlockComment { .. })
}

fn line_number_at(source: &str, offset: usize) -> usize {
    source[..offset].matches('\n').count() + 1
}

#[cfg(test)]
mod tests {
    use super::comment_lines;

    #[test]
    fn finds_line_block_and_doc_comments() {
        let source = "fn a() {}\n// note\n/* block */\n/// doc\n//! inner\nfn b() {}\n";
        assert_eq!(comment_lines(source), vec![2, 3, 4, 5]);
    }

    #[test]
    fn ignores_comment_markers_inside_strings() {
        let source = "fn a() -> &'static str { \"http://example.com /* not */\" }\n";
        assert_eq!(comment_lines(source), Vec::<usize>::new());
    }
}
