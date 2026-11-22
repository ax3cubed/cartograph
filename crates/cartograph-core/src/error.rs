//! The error contract. Every crate defines its own `thiserror` enum and implements [`Explain`], so
//! any error prints three lines (what, why, try, per E1) and maps to one of four exit codes (E2).

use std::fmt;

/// Process exit codes (E2). Code 4 (lock busy) and 5 (format too new) no longer exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExitCode {
    /// Done.
    Ok = 0,
    /// Any error, including an index format that is too new.
    Error = 1,
    /// Bad usage: an unknown flag, a missing argument, a malformed value.
    Usage = 2,
    /// `status --check` found a repo behind its indexed commit.
    Behind = 3,
}

impl From<ExitCode> for i32 {
    fn from(code: ExitCode) -> i32 {
        code as i32
    }
}

/// What every error in every crate can say about itself.
pub trait Explain: std::error::Error {
    /// What went wrong, in one line.
    fn what(&self) -> String {
        self.to_string()
    }
    /// Why it went wrong.
    fn why(&self) -> String;
    /// What to try next. One concrete action, ideally a command.
    fn action(&self) -> String;
    /// The exit code a command should end with.
    fn exit_code(&self) -> ExitCode {
        ExitCode::Error
    }
}

/// A rendered error, detached from the crate that raised it. The CLI prints it; the HTTP layer turns
/// it into `{"error":{"code","what","why","try"}}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub what: String,
    pub why: String,
    pub action: String,
    pub exit_code: ExitCode,
}

impl Report {
    pub fn from_error(e: &dyn Explain) -> Self {
        Self {
            what: e.what(),
            why: e.why(),
            action: e.action(),
            exit_code: e.exit_code(),
        }
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "error: {}", self.what)?;
        writeln!(f, "  why: {}", self.why)?;
        write!(f, "  try: {}", self.action)
    }
}

/// Errors raised by this crate.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("no home folder found")]
    NoHome,
    #[error("the data folder {path} is inside a sync client's folder")]
    SyncedDataDir { path: String },
}

impl Explain for CoreError {
    fn why(&self) -> String {
        match self {
            Self::NoHome => "neither HOME nor USERPROFILE is set, so ~/.cartograph has no location".into(),
            Self::SyncedDataDir { .. } => {
                "sync clients upload SQLite's -wal and -shm files separately, which corrupts the index (B1)".into()
            }
        }
    }
    fn action(&self) -> String {
        match self {
            Self::NoHome => "set CARTOGRAPH_HOME to a local folder".into(),
            Self::SyncedDataDir { .. } => {
                "set CARTOGRAPH_HOME or index_path to a folder outside the synced one".into()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_are_the_four_in_e2() {
        assert_eq!(i32::from(ExitCode::Ok), 0);
        assert_eq!(i32::from(ExitCode::Error), 1);
        assert_eq!(i32::from(ExitCode::Usage), 2);
        assert_eq!(i32::from(ExitCode::Behind), 3);
        assert_eq!(CoreError::NoHome.exit_code(), ExitCode::Error);
    }

    #[test]
    fn an_error_renders_as_three_lines() {
        let report = Report::from_error(&CoreError::SyncedDataDir {
            path: "~/Dropbox/.cartograph".into(),
        });
        assert_eq!(report.to_string().lines().count(), 3);
        insta::assert_snapshot!(report.to_string());
    }
}
