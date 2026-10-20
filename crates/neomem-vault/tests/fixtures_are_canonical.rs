#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

use neomem_vault::{parse, render};

fn fixture_files(directory: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).unwrap().filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            fixture_files(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "md") {
            found.push(path);
        }
    }
}

#[test]
fn every_vault_fixture_renders_back_byte_for_byte() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.agents/skills/neomem-control/fixtures/vault");
    let mut files = Vec::new();
    fixture_files(&root, &mut files);
    assert!(!files.is_empty(), "no vault fixtures found under {}", root.display());
    for file in files {
        let text = fs::read_to_string(&file).unwrap();
        let memory = parse(&text).unwrap_or_else(|error| panic!("{}: {error}", file.display()));
        assert_eq!(render(&memory).unwrap(), text, "{} is not canonical", file.display());
    }
}
