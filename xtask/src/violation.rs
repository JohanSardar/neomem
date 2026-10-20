use std::fmt;
use std::path::PathBuf;

#[derive(Debug)]
pub struct Violation {
    pub check: &'static str,
    pub file: PathBuf,
    pub line: usize,
    pub message: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "[{}] {}:{}: {}",
            self.check,
            self.file.display(),
            self.line,
            self.message
        )
    }
}
