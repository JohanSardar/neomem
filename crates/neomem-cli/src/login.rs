use std::process::{Command, Stdio};
use std::time::Duration;

use neomem_proto::{Home, Request, Response};

use crate::args::LoginArgs;
use crate::connection::DaemonConnection;

const SIGN_IN_WAIT: Duration = Duration::from_secs(330);

pub fn run(home: &Home, arguments: &LoginArgs) -> anyhow::Result<Response> {
    let provider = arguments.provider.name().to_owned();
    let opened = DaemonConnection::open(home)?.call(Request::Login { provider: provider.clone() })?;
    let Response::LoginOpened { url, .. } = &opened else { return Ok(opened) };
    eprintln!("Sign in to {provider} in your browser:\n{url}");
    if !arguments.no_browser {
        open_in_browser(url);
    }
    DaemonConnection::open(home)?.waiting_up_to(SIGN_IN_WAIT)?.call(Request::LoginFinish { provider })
}

fn open_in_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let launched = Command::new(opener)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if launched.is_err() {
        eprintln!("Open the address above in a browser to continue.");
    }
}
