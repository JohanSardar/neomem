use std::io;
use std::process::ExitCode;

use neomem_testkit::fake_process;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let outcome = match arguments.as_slice() {
        [flag, brain, mode] if flag == "--brain" && mode == "command" => {
            fake_process::answer_command(io::stdin().lock(), io::stdout().lock(), brain)
        }
        [flag, brain, mode] if flag == "--brain" && mode == "codex-app-server" => {
            fake_process::serve_codex_app_server(io::stdin().lock(), io::stdout().lock(), brain)
        }
        _ => {
            eprintln!("usage: neomem-fake-llm --brain <fake model service URL> command|codex-app-server");
            return ExitCode::from(2);
        }
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neomem-fake-llm: {error}");
            ExitCode::FAILURE
        }
    }
}
