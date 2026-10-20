use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::Context;
use cargo_metadata::{DependencyKind, Metadata, MetadataCommand, PackageId};

use crate::violation::Violation;

const TESTKIT: &str = "neomem-testkit";

#[derive(Debug)]
struct Rule {
    crate_name: &'static str,
    may_depend_on: &'static [&'static str],
    only_external: Option<&'static [&'static str]>,
    never_pulls_in: &'static [&'static str],
}

const RULES: &[Rule] = &[
    Rule {
        crate_name: "neomem-core",
        may_depend_on: &[],
        only_external: Some(&["serde", "thiserror"]),
        never_pulls_in: &[],
    },
    Rule::on("neomem-proto", &["neomem-core"]),
    Rule::on("neomem-vault", &["neomem-core"]),
    Rule::on("neomem-state", &["neomem-core"]),
    Rule::on("neomem-index", &["neomem-core"]),
    Rule::on("neomem-models", &["neomem-core"]),
    Rule::on("neomem-capture", &["neomem-core"]),
    Rule::on("neomem-testkit", &["neomem-core"]),
    Rule::on(
        "neomem-engine",
        &["neomem-core", "neomem-proto", "neomem-vault", "neomem-state", "neomem-index"],
    ),
    Rule::on(
        "neomemd",
        &[
            "neomem-core",
            "neomem-proto",
            "neomem-vault",
            "neomem-state",
            "neomem-index",
            "neomem-models",
            "neomem-capture",
            "neomem-engine",
        ],
    ),
    Rule {
        crate_name: "neomem-cli",
        may_depend_on: &["neomem-core", "neomem-proto", "neomem-state"],
        only_external: None,
        never_pulls_in: &[
            "tokio",
            "llama-cpp-2",
            "usearch",
            "neomem-index",
            "neomem-models",
            "neomem-engine",
        ],
    },
    Rule::on("neomem-eval", &["neomem-core", "neomem-proto"]),
    Rule::on("neomem-control", &["neomem-core", "neomem-proto", TESTKIT]),
    Rule::on("xtask", &[]),
];

impl Rule {
    const fn on(crate_name: &'static str, may_depend_on: &'static [&'static str]) -> Self {
        Self { crate_name, may_depend_on, only_external: None, never_pulls_in: &[] }
    }
}

#[derive(Debug, Default)]
struct CrateDependencies {
    manifest: PathBuf,
    workspace_normal: BTreeSet<String>,
    workspace_dev: BTreeSet<String>,
    external_normal: BTreeSet<String>,
    reachable_normal: BTreeSet<String>,
}

pub fn check(root: &Path) -> anyhow::Result<Vec<Violation>> {
    let metadata = MetadataCommand::new()
        .manifest_path(root.join("Cargo.toml"))
        .exec()
        .context("running cargo metadata")?;
    let crates = collect(&metadata, root)?;
    Ok(evaluate(&crates))
}

fn collect(metadata: &Metadata, root: &Path) -> anyhow::Result<BTreeMap<String, CrateDependencies>> {
    let members: BTreeSet<String> =
        metadata.workspace_packages().iter().map(|package| package.name.to_string()).collect();
    let mut crates = BTreeMap::new();
    for package in metadata.workspace_packages() {
        let mut facts = CrateDependencies {
            manifest: package.manifest_path.strip_prefix(root).unwrap_or(&package.manifest_path).into(),
            reachable_normal: reachable_without_dev(metadata, &package.id)?,
            ..CrateDependencies::default()
        };
        for dependency in &package.dependencies {
            let is_member = members.contains(&dependency.name);
            match (dependency.kind, is_member) {
                (DependencyKind::Development, true) => facts.workspace_dev.insert(dependency.name.clone()),
                (DependencyKind::Development, false) => false,
                (_, true) => facts.workspace_normal.insert(dependency.name.clone()),
                (_, false) => facts.external_normal.insert(dependency.name.clone()),
            };
        }
        crates.insert(package.name.to_string(), facts);
    }
    Ok(crates)
}

fn reachable_without_dev(metadata: &Metadata, start: &PackageId) -> anyhow::Result<BTreeSet<String>> {
    let resolve = metadata.resolve.as_ref().context("cargo metadata returned no resolve graph")?;
    let nodes: BTreeMap<&PackageId, _> = resolve.nodes.iter().map(|node| (&node.id, node)).collect();
    let mut seen = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut pending = vec![start];
    while let Some(id) = pending.pop() {
        let Some(node) = nodes.get(id) else { continue };
        for edge in &node.deps {
            let is_runtime = edge.dep_kinds.iter().any(|info| info.kind != DependencyKind::Development);
            if is_runtime && seen.insert(&edge.pkg) {
                names.insert(metadata[&edge.pkg].name.to_string());
                pending.push(&edge.pkg);
            }
        }
    }
    Ok(names)
}

fn evaluate(crates: &BTreeMap<String, CrateDependencies>) -> Vec<Violation> {
    let mut violations = Vec::new();
    for (name, facts) in crates {
        let Some(rule) = RULES.iter().find(|rule| rule.crate_name == name) else {
            violations.push(violation(facts, format!("{name} has no dependency rule in xtask")));
            continue;
        };
        violations.extend(breaches(rule, facts).into_iter().map(|message| violation(facts, message)));
    }
    violations
}

fn breaches(rule: &Rule, facts: &CrateDependencies) -> Vec<String> {
    let name = rule.crate_name;
    let forbidden_internal = facts
        .workspace_normal
        .iter()
        .filter(|dependency| !rule.may_depend_on.contains(&dependency.as_str()))
        .map(|dependency| format!("{name} may not depend on {dependency}"));
    let forbidden_dev = facts
        .workspace_dev
        .iter()
        .filter(|dependency| {
            dependency.as_str() != TESTKIT && !rule.may_depend_on.contains(&dependency.as_str())
        })
        .map(|dependency| format!("{name} may not use {dependency} even in tests"));
    let forbidden_external = facts
        .external_normal
        .iter()
        .filter(|dependency| {
            rule.only_external.is_some_and(|allowed| !allowed.contains(&dependency.as_str()))
        })
        .map(|dependency| {
            format!("{name} may only use {:?} from outside, not {dependency}", rule.only_external)
        });
    let forbidden_transitive = rule
        .never_pulls_in
        .iter()
        .filter(|banned| facts.reachable_normal.contains(**banned))
        .map(|banned| format!("{name} must never pull in {banned}"));
    forbidden_internal.chain(forbidden_dev).chain(forbidden_external).chain(forbidden_transitive).collect()
}

fn violation(facts: &CrateDependencies, message: String) -> Violation {
    Violation { check: "dependency-rules", file: facts.manifest.clone(), line: 1, message }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crate_with(normal: &[&str], external: &[&str], reachable: &[&str]) -> CrateDependencies {
        CrateDependencies {
            manifest: PathBuf::from("Cargo.toml"),
            workspace_normal: normal.iter().map(ToString::to_string).collect(),
            workspace_dev: BTreeSet::new(),
            external_normal: external.iter().map(ToString::to_string).collect(),
            reachable_normal: reachable.iter().map(ToString::to_string).collect(),
        }
    }

    fn messages(name: &str, facts: CrateDependencies) -> Vec<String> {
        let crates = BTreeMap::from([(name.to_owned(), facts)]);
        evaluate(&crates).into_iter().map(|violation| violation.message).collect()
    }

    #[test]
    fn core_may_not_reach_into_the_workspace_or_beyond_serde() {
        let found = messages("neomem-core", crate_with(&["neomem-proto"], &["serde", "tokio"], &[]));
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn engine_may_not_name_a_concrete_model_adapter() {
        let found = messages("neomem-engine", crate_with(&["neomem-core", "neomem-models"], &[], &[]));
        assert_eq!(found, vec!["neomem-engine may not depend on neomem-models".to_owned()]);
    }

    #[test]
    fn the_thin_client_never_pulls_in_the_async_runtime() {
        let found = messages("neomem-cli", crate_with(&["neomem-proto"], &[], &["tokio"]));
        assert_eq!(found, vec!["neomem-cli must never pull in tokio".to_owned()]);
    }

    #[test]
    fn unknown_crates_need_a_rule() {
        let found = messages("neomem-surprise", crate_with(&[], &[], &[]));
        assert_eq!(found, vec!["neomem-surprise has no dependency rule in xtask".to_owned()]);
    }
}
