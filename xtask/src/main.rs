mod no_comments;
mod violation;
mod workspace;

use std::process::ExitCode;

use violation::Violation;

fn main() -> ExitCode {
    if std::env::args().nth(1).as_deref() == Some("check") {
        report(run_every_check())
    } else {
        eprintln!("usage: cargo xtask check");
        ExitCode::FAILURE
    }
}

fn run_every_check() -> anyhow::Result<Vec<Violation>> {
    let root = workspace::root()?;
    let mut violations = Vec::new();
    violations.extend(no_comments::check(&root)?);
    Ok(violations)
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
