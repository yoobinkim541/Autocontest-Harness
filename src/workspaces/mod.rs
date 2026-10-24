//! Where contest sessions run (the ADE/terminal side). Contract: `docs/CONTRACT.md`.
//! - `orca`: Orca workspaces and terminals (`orca` CLI).
//! - `tmux`: one tmux session (`contest-harness`), one window per contest.
//! - `headless`: a detached background process; no TTY, so the runner's one-shot command is used.
//! - `terminal`: replace the current process (used by `ach chat`).

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::Config;
use crate::error::Result;
use crate::registry::Entry;

pub mod headless;
pub mod orca;
pub mod terminal;
pub mod tmux;

/// What to open: run `argv` in `cwd`, titled `title`, inside `workspace_id` when the backend has one.
pub struct Open<'a> {
    pub cwd: PathBuf,
    pub title: &'a str,
    pub argv: Vec<String>,
    pub workspace_id: Option<&'a str>,
}

pub trait Workspace: Sync {
    fn name(&self) -> &'static str;
    /// The backend's executable is on PATH (always true for headless and terminal).
    fn available(&self) -> bool;
    /// False when sessions have no TTY and must use the runner's one-shot command.
    fn interactive(&self) -> bool;
    /// Create (or reuse) the workspace for a contest. `None` when the backend has no workspaces.
    fn create(&self, entry: &Entry, config: &Config) -> Result<Option<String>>;
    /// Start the session and return a handle that `tail` understands.
    fn open(&self, spec: &Open<'_>) -> Result<String>;
    /// The last `lines` lines of the session's screen or log.
    fn tail(&self, handle: &str, lines: usize) -> Result<String>;
    /// Stop the session behind `handle`. A session that is already gone is not an error.
    fn close(&self, handle: &str) -> Result<()>;
    /// Forget the workspace (never the contest folder itself; the harness trashes that). Backends
    /// without workspaces do nothing.
    fn remove_workspace(&self, workspace_id: &str) -> Result<()>;
}

pub const NAMES: &[&str] = &["orca", "tmux", "headless", "terminal"];

pub fn get(name: &str) -> Result<&'static dyn Workspace> {
    Ok(match name {
        "orca" => &orca::Orca,
        "tmux" => &tmux::Tmux,
        "headless" => &headless::Headless,
        "terminal" => &terminal::Terminal,
        _ => crate::bail!("알 수 없는 작업 환경: {name} (가능: {})", NAMES.join(", ")),
    })
}

/// Quote one word for a POSIX shell (zsh and sh agree on single quotes).
pub fn shell_quote(word: &str) -> String {
    if !word.is_empty() && word.bytes().all(|b| b.is_ascii_alphanumeric() || b"@%+=:,./_-".contains(&b)) {
        return word.to_string();
    }
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// `argv` as one shell line.
pub fn shell_join(argv: &[String]) -> String {
    argv.iter().map(|w| shell_quote(w)).collect::<Vec<_>>().join(" ")
}

/// The line a terminal types to run `argv` in `cwd`. `argv` may start with `env NAME=value…`
/// ([`crate::launch::session_env`]). On Unix this is `cd <cwd> && <argv>` (pi, claude and dsh have
/// no cwd flag); on Windows it is the same in PowerShell, which Orca terminals there run.
pub fn shell_command(cwd: &Path, argv: &[String]) -> String {
    if cfg!(windows) {
        return powershell_command(cwd, argv);
    }
    format!("cd {} && {}", shell_quote(&cwd.to_string_lossy()), shell_join(argv))
}

/// Quote one word for PowerShell: a single-quoted string, in which only quote characters (ASCII
/// and the typographic ones PowerShell also accepts) need doubling.
pub fn powershell_quote(word: &str) -> String {
    let mut out = String::with_capacity(word.len() + 2);
    out.push('\'');
    for c in word.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// `Set-Location -LiteralPath <cwd>; if ($?) { $env:NAME = <value>; … & <program> <args> }`: the
/// variables are set only when the folder exists, and every word is quoted, so nothing in it is
/// read as PowerShell syntax (`@`, `$`, `;`).
pub fn powershell_command(cwd: &Path, argv: &[String]) -> String {
    let (env, command) = split_env(argv);
    let mut line = format!("Set-Location -LiteralPath {}; if ($?) {{ ", powershell_quote(&cwd.to_string_lossy()));
    for (name, value) in env {
        line.push_str(&format!("$env:{name} = {}; ", powershell_quote(value)));
    }
    line.push('&');
    for word in command {
        line.push(' ');
        line.push_str(&powershell_quote(word));
    }
    line.push_str(" }");
    line
}

/// `env NAME=value… program args…` split into the assignments and the command; an argv without
/// the `env` prefix has no assignments.
pub fn split_env(argv: &[String]) -> (Vec<(&str, &str)>, &[String]) {
    let Some(("env", rest)) = argv.split_first().map(|(head, rest)| (head.as_str(), rest)) else {
        return (Vec::new(), argv);
    };
    let count = rest.iter().take_while(|w| w.split_once('=').is_some_and(|(name, _)| is_env_name(name))).count();
    let env = rest[..count].iter().filter_map(|w| w.split_once('=')).collect();
    (env, &rest[count..])
}

fn is_env_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// A [`Command`] for `argv`, with the `env NAME=value` prefix applied as environment variables
/// rather than run through an `env` program (Windows has none).
pub fn command(argv: &[String]) -> Result<Command> {
    let (env, words) = split_env(argv);
    let Some((program, args)) = words.split_first() else { crate::bail!("실행할 명령이 비었다") };
    let mut command = Command::new(program);
    command.args(args).envs(env);
    Ok(command)
}

/// Run a backend CLI (bounded by [`crate::process::LOCAL`]) and return stdout, failing with its stderr.
pub(crate) fn run(program: &str, args: &[&str]) -> Result<String> {
    let out = crate::process::output(program, args, None, crate::process::LOCAL)?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.lines().filter(|l| !l.contains("codesign")).collect::<Vec<_>>().join("\n");
        crate::bail!("{program} {} 실패: {}", args.join(" "), err.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
