use super::{Open, Workspace, command, split_env};
use crate::config::Config;
use crate::error::{Error, Result};
use crate::registry::Entry;

pub struct Terminal;

impl Workspace for Terminal {
    fn name(&self) -> &'static str {
        "terminal"
    }

    fn available(&self) -> bool {
        true
    }

    fn interactive(&self) -> bool {
        true
    }

    fn create(&self, _entry: &Entry, _config: &Config) -> Result<Option<String>> {
        Ok(None)
    }

    /// Replaces the harness process on Unix, so it only returns on failure. Windows has no exec:
    /// the command runs on this console, Ctrl+C reaches it alone, and the harness exits with its
    /// exit code.
    fn open(&self, spec: &Open<'_>) -> Result<String> {
        let program = split_env(&spec.argv).1.first().cloned().unwrap_or_default();
        let mut command = command(&spec.argv)?;
        command.current_dir(&spec.cwd);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            let err = command.exec();
            Err(Error(format!("{program} 실행 실패: {err}")))
        }
        #[cfg(windows)]
        {
            crate::platform::ignore_console_interrupts();
            let status = command.status().map_err(|err| Error(format!("{program} 실행 실패: {err}")))?;
            std::process::exit(status.code().unwrap_or(1))
        }
    }

    fn tail(&self, _handle: &str, _lines: usize) -> Result<String> {
        crate::bail!("terminal 작업 환경은 화면을 읽을 수 없다(그 터미널에서 직접 본다)")
    }

    fn close(&self, _handle: &str) -> Result<()> {
        Ok(())
    }

    fn remove_workspace(&self, _workspace_id: &str) -> Result<()> {
        Ok(())
    }
}
