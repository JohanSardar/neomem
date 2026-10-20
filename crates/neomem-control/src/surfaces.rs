use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::Context;
use syn::visit::Visit;

const SURFACE_KINDS: &[&str] = &["neomem", "http", "hook", "mcp", "pi"];

pub fn in_code(root: &Path) -> anyhow::Result<BTreeSet<String>> {
    let mut surfaces = BTreeSet::new();
    surfaces.extend(cli_commands(&root.join("crates/neomem-cli/src/args.rs"))?);
    surfaces.extend(http_routes(&root.join("crates/neomemd/src"))?);
    Ok(surfaces)
}

pub fn is_checked_surface(surface: &str) -> bool {
    surface.split_whitespace().next().is_some_and(|kind| SURFACE_KINDS.contains(&kind))
}

pub fn graph_surface_exists(graph_surface: &str, code: &BTreeSet<String>) -> bool {
    let words: Vec<&str> = graph_surface.split_whitespace().collect();
    match words.as_slice() {
        ["neomem", command, ..] => code.contains(&format!("neomem {command}")),
        ["http", path] => code.iter().any(|surface| surface.starts_with("http ") && surface.ends_with(path)),
        _ => code.contains(graph_surface),
    }
}

pub fn code_surface_is_listed(code_surface: &str, graph_surfaces: &BTreeSet<&str>) -> bool {
    graph_surfaces.iter().any(|graph_surface| {
        let single = BTreeSet::from([code_surface.to_owned()]);
        graph_surface_exists(graph_surface, &single)
    })
}

fn parse(path: &Path) -> anyhow::Result<Option<syn::File>> {
    if !path.exists() {
        return Ok(None);
    }
    let source = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Some(syn::parse_file(&source).with_context(|| format!("parsing {}", path.display()))?))
}

fn cli_commands(args_file: &Path) -> anyhow::Result<Vec<String>> {
    let Some(file) = parse(args_file)? else { return Ok(Vec::new()) };
    let commands = file
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Enum(found) if found.ident == "Command" => Some(found),
            _ => None,
        })
        .flat_map(|command| command.variants.iter())
        .map(|variant| format!("neomem {}", kebab_case(&variant.ident.to_string())))
        .collect();
    Ok(commands)
}

fn http_routes(source_dir: &Path) -> anyhow::Result<Vec<String>> {
    let mut finder = RouteFinder::default();
    for entry in fs::read_dir(source_dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "rs")
            && let Some(file) = parse(&path)?
        {
            finder.visit_file(&file);
        }
    }
    Ok(finder.routes)
}

#[derive(Default)]
struct RouteFinder {
    routes: Vec<String>,
}

impl<'ast> Visit<'ast> for RouteFinder {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "route"
            && let (Some(syn::Expr::Lit(path)), Some(syn::Expr::Call(handler))) =
                (call.args.first(), call.args.get(1))
            && let syn::Lit::Str(path) = &path.lit
            && let syn::Expr::Path(method) = handler.func.as_ref()
            && let Some(method) = method.path.segments.last()
        {
            self.routes.push(format!("http {} {}", method.ident.to_string().to_uppercase(), path.value()));
        }
        syn::visit::visit_expr_method_call(self, call);
    }
}

fn kebab_case(identifier: &str) -> String {
    let mut kebab = String::new();
    for (index, character) in identifier.chars().enumerate() {
        if character.is_uppercase() && index > 0 {
            kebab.push('-');
        }
        kebab.extend(character.to_lowercase());
    }
    kebab
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_cli_surfaces_match_their_top_level_command() {
        let code = BTreeSet::from(["neomem recall".to_owned()]);
        assert!(graph_surface_exists("neomem recall on", &code));
        assert!(!graph_surface_exists("neomem search", &code));
    }

    #[test]
    fn variant_names_become_kebab_case_commands() {
        assert_eq!(kebab_case("Reextract"), "reextract");
        assert_eq!(kebab_case("SubagentStop"), "subagent-stop");
    }
}
