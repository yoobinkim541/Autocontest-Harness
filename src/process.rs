//! Bounded external commands: every git/gh/orca/tmux call goes through [`output`] so a hung
//! credential prompt or network stall cannot block the harness forever.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::LazyLock;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};

/// Budget for commands that talk to the network (git fetch/push, gh).
pub const NETWORK: Duration = Duration::from_secs(120);
/// Budget for local commands (git status, tmux, orca).
pub const LOCAL: Duration = Duration::from_secs(30);

/// The user configured `core.sshCommand` (an identity file or proxy); `GIT_SSH_COMMAND` would
/// override it, so it is left alone then.
static USER_SSH_COMMAND: LazyLock<bool> = LazyLock::new(|| {
    Command::new("git")
        .args(["config", "--get", "core.sshCommand"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|out| out.status.success() && !out.stdout.trim_ascii().is_empty())
});

/// Runs `program args` with stdin closed and stdout/stderr captured. Git is told never to prompt.
/// The child runs in its own process group (Unix) or console-less process group (Windows); the
/// whole group or process tree is killed when it exceeds `timeout`.
pub fn output(program: &str, args: &[&str], cwd: Option<&Path>, timeout: Duration) -> Result<Output> {
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    crate::platform::own_group(&mut command);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    command.env("GIT_TERMINAL_PROMPT", "0");
    if std::env::var_os("GIT_SSH_COMMAND").is_none() && !*USER_SSH_COMMAND {
        command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes -o ConnectTimeout=15");
    }
    let mut child = command.spawn().map_err(|err| Error(format!("{program} 실행 실패: {err}")))?;
    let pid = child.id();
    // Drain the pipes on threads so a chatty child cannot fill a pipe buffer and stall.
    let stdout = child.stdout.take().map(drain);
    let stderr = child.stderr.take().map(drain);
    let started = Instant::now();
    let timed_out = || Error(format!("{program} {}: {}초 안에 끝나지 않아 멈췄다", args.join(" "), timeout.as_secs()));
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= timeout {
            crate::platform::kill_tree(pid);
            let _ = child.kill();
            let _ = child.wait();
            return Err(timed_out());
        }
        thread::sleep(Duration::from_millis(20));
    };
    // A grandchild (credential helper, ssh master) may still hold the pipes open: wait for the
    // output only within what is left of the budget, then take what was read.
    let collect = |pipe: Option<Receiver<Vec<u8>>>| {
        let left = timeout.saturating_sub(started.elapsed()).max(Duration::from_millis(100));
        pipe.and_then(|rx| rx.recv_timeout(left).ok()).unwrap_or_default()
    };
    let stdout = collect(stdout);
    let stderr = collect(stderr);
    Ok(Output { status, stdout, stderr })
}

fn drain(mut pipe: impl Read + Send + 'static) -> Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    rx
}
