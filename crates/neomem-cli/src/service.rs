use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use neomem_proto::{Home, publish_file};

use crate::connection::daemon_binary;

const SYSTEMD_UNIT: &str = "neomemd.service";
const LAUNCHD_LABEL: &str = "dev.neomem.neomemd";

pub enum Service {
    Systemd { unit: PathBuf },
    Launchd { plist: PathBuf },
}

impl Service {
    pub fn for_this_machine() -> Option<Self> {
        let user_home = env::var_os("HOME").map(PathBuf::from)?;
        if cfg!(target_os = "macos") {
            let plist = user_home.join("Library/LaunchAgents").join(format!("{LAUNCHD_LABEL}.plist"));
            return Some(Self::Launchd { plist });
        }
        let config = env::var_os("XDG_CONFIG_HOME").map_or_else(|| user_home.join(".config"), PathBuf::from);
        Some(Self::Systemd { unit: config.join("systemd/user").join(SYSTEMD_UNIT) })
    }

    pub fn serving(home: &Home) -> Option<Self> {
        Self::for_this_machine().filter(|service| service.serves(home))
    }

    pub fn file(&self) -> &Path {
        match self {
            Self::Systemd { unit } => unit,
            Self::Launchd { plist } => plist,
        }
    }

    fn serves(&self, home: &Home) -> bool {
        fs::read_to_string(self.file()).is_ok_and(|text| text.contains(&self.home_setting(home)))
    }

    fn home_setting(&self, home: &Home) -> String {
        match self {
            Self::Systemd { .. } => format!("Environment=\"NEOMEM_HOME={}\"", home.root().display()),
            Self::Launchd { .. } => {
                format!(
                    "<key>NEOMEM_HOME</key>\n    <string>{}</string>",
                    xml_text(&home.root().display().to_string())
                )
            }
        }
    }

    pub fn is_running(&self) -> bool {
        match self {
            Self::Systemd { .. } => {
                succeeded(&quietly("systemctl", &["--user", "is-active", "--quiet", SYSTEMD_UNIT]))
            }
            Self::Launchd { .. } => succeeded(&quietly("launchctl", &["print", &launchd_target()])),
        }
    }

    pub fn install(&self, home: &Home) -> Result<(), String> {
        let text = match self {
            Self::Systemd { .. } => systemd_unit(&self.home_setting(home), &home.daemon_log_path()),
            Self::Launchd { .. } => launchd_plist(&self.home_setting(home), &home.daemon_log_path()),
        };
        let file = self.file();
        fs::create_dir_all(home.run_dir()).map_err(|error| error.to_string())?;
        file.parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| publish_file(file, text))
            .map_err(|error| error.to_string())?;
        match self {
            Self::Systemd { .. } => {
                run("systemctl", &["--user", "daemon-reload"])?;
                run("systemctl", &["--user", "enable", SYSTEMD_UNIT])?;
                run("systemctl", &["--user", "restart", SYSTEMD_UNIT])
            }
            Self::Launchd { plist } => {
                let _ = quietly("launchctl", &["bootout", &launchd_target()]);
                run("launchctl", &["bootstrap", &launchd_domain(), &plist.display().to_string()])
            }
        }
    }

    pub fn start(&self) -> bool {
        match self {
            Self::Systemd { .. } => succeeded(&quietly("systemctl", &["--user", "start", SYSTEMD_UNIT])),
            Self::Launchd { .. } => succeeded(&quietly("launchctl", &["kickstart", &launchd_target()])),
        }
    }
}

fn systemd_unit(home_setting: &str, log: &Path) -> String {
    format!(
        "[Unit]\nDescription=neomem memory daemon\n\n[Service]\nExecStart={}\n{home_setting}\nStandardOutput=append:{}\nStandardError=inherit\nRestart=on-failure\n\n[Install]\nWantedBy=default.target\n",
        daemon_binary().display(),
        log.display()
    )
}

fn launchd_plist(home_setting: &str, log: &Path) -> String {
    let log = xml_text(&log.display().to_string());
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key>\n  <string>{LAUNCHD_LABEL}</string>\n  <key>ProgramArguments</key>\n  <array>\n    <string>{}</string>\n  </array>\n  <key>EnvironmentVariables</key>\n  <dict>\n    {home_setting}\n  </dict>\n  <key>StandardOutPath</key>\n  <string>{log}</string>\n  <key>StandardErrorPath</key>\n  <string>{log}</string>\n  <key>RunAtLoad</key>\n  <true/>\n  <key>KeepAlive</key>\n  <true/>\n</dict>\n</plist>\n",
        xml_text(&daemon_binary().display().to_string())
    )
}

fn xml_text(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn launchd_domain() -> String {
    let user = quietly("id", &["-u"]).map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
    format!("gui/{}", user.unwrap_or_default())
}

fn launchd_target() -> String {
    format!("{}/{LAUNCHD_LABEL}", launchd_domain())
}

pub fn quietly(program: &str, arguments: &[&str]) -> io::Result<Output> {
    Command::new(program).args(arguments).stdin(Stdio::null()).output()
}

pub fn succeeded(output: &io::Result<Output>) -> bool {
    output.as_ref().is_ok_and(|output| output.status.success())
}

pub fn described(output: &io::Result<Output>) -> String {
    match output {
        Ok(output) => {
            let text = if output.stderr.is_empty() { &output.stdout } else { &output.stderr };
            String::from_utf8_lossy(text).trim().lines().last().unwrap_or_default().to_owned()
        }
        Err(error) => error.to_string(),
    }
}

fn run(program: &str, arguments: &[&str]) -> Result<(), String> {
    let output = quietly(program, arguments);
    if succeeded(&output) {
        Ok(())
    } else {
        Err(format!("{program} {}: {}", arguments.join(" "), described(&output)))
    }
}
