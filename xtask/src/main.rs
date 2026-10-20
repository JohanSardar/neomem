mod ci;
mod dependency_rules;
mod no_comments;
mod violation;
mod workspace;

use std::process::ExitCode;

use violation::Violation;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("check") => report(run_every_check()),
        Some("ci") => finish(workspace::root().and_then(|root| ci::run_every_step(&root))),
        _ => {
            eprintln!("usage: cargo xtask check|ci");
            ExitCode::FAILURE
        }
    }
}

fn run_every_check() -> anyhow::Result<Vec<Violation>> {
    let root = workspace::root()?;
    let mut violations = Vec::new();
    violations.extend(no_comments::check(&root)?);
    violations.extend(dependency_rules::check(&root)?);
    Ok(violations)
}

fn finish(outcome: anyhow::Result<()>) -> ExitCode {
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

fn report(outcome: anyhow::Result<Vec<Violation>>) -> ExitCode {
    match outcome {
        Ok(violations) if violations.is_empty() => {
            eprintln!("xtask check: all checks passed");
            ExitCode::SUCCESS
        }
        Ok(violations) => {
            for violation in &violations {
                eprintln!("{violation}");
            }
            eprintln!("xtask check: {} violation(s)", violations.len());
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("xtask check could not run: {error:#}");
            ExitCode::FAILURE
        }
    }
}
