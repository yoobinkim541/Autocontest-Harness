//! Where the harness keeps its files. The repository root is `ACH_HOME` when set, otherwise the
//! checkout that holds the running binary (`<clone>/bin/ach`, `<clone>/target/release/ach`),
//! otherwise the checkout the binary was built from. `main` refuses to run when that root is not a
//! harness checkout ([`checkout_ok`]).

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// A file every harness checkout has; its presence is what makes a directory the root.
pub const MARKER: &str = "harness/prompts/launch.md";

static BINARY_ROOT: LazyLock<PathBuf> = LazyLock::new(|| {
    let exe = std::env::current_exe().and_then(|exe| canonicalize(&exe)).ok();
    exe.and_then(|exe| exe.ancestors().skip(1).find(|dir| checkout_ok(dir)).map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
});

pub fn root() -> PathBuf {
    match std::env::var_os("ACH_HOME").filter(|home| !home.is_empty()) {
        Some(home) => PathBuf::from(home),
        None => BINARY_ROOT.clone(),
    }
}

/// `root` is a harness checkout (it has [`MARKER`]).
pub fn checkout_ok(root: &Path) -> bool {
    root.join(MARKER).is_file()
}

/// The root, or an error naming it when it is not a harness checkout (moved or deleted clone, typo
/// in `ACH_HOME`), so no command silently starts an empty registry somewhere else.
pub fn checked_root() -> crate::error::Result<PathBuf> {
    let root = root();
    if checkout_ok(&root) {
        return Ok(root);
    }
    let source = if std::env::var_os("ACH_HOME").is_some_and(|h| !h.is_empty()) {
        "ACH_HOME"
    } else {
        "실행 파일 위치나 빌드한 클론"
    };
    crate::bail!(
        "하네스 클론을 찾지 못했다: {}에 {MARKER}가 없다({source} 기준). ACH_HOME을 클론 경로로 지정하거나 클론에서 scripts/install.sh로 다시 설치한다",
        root.display()
    )
}

/// The directory of the running `ach` (`<clone>/bin`). It is not on the user's PATH; sessions the
/// harness starts get it through [`session_path`].
pub fn bin_dir() -> PathBuf {
    std::env::current_exe()
        .and_then(|exe| canonicalize(&exe))
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| root().join("bin"))
}

/// The `ach` executable in [`bin_dir`] (`ach.exe` on Windows).
pub fn ach_exe() -> PathBuf {
    bin_dir().join(format!("ach{}", std::env::consts::EXE_SUFFIX))
}

/// Separator between `PATH` entries.
pub const PATH_SEPARATOR: char = if cfg!(windows) { ';' } else { ':' };

/// `PATH` for a session the harness starts: [`bin_dir`] first, so its agents can call `ach`.
pub fn session_path() -> String {
    let bin = bin_dir().display().to_string();
    match std::env::var("PATH") {
        Ok(path) if path.split(PATH_SEPARATOR).next() == Some(bin.as_str()) => path,
        Ok(path) if !path.is_empty() => format!("{bin}{PATH_SEPARATOR}{path}"),
        _ => bin,
    }
}

/// `HOME`, or on Windows `USERPROFILE` (shells there rarely set `HOME`).
pub fn home() -> PathBuf {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty());
    let home = if cfg!(windows) { home.or_else(|| std::env::var_os("USERPROFILE")) } else { home };
    home.map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// `~` and `~/...` (on Windows also `~\...`) expand to the home directory; everything else is
/// taken as is.
pub fn expand(path: &str) -> PathBuf {
    let separator = |c: char| c == '/' || (cfg!(windows) && c == '\\');
    match path.strip_prefix('~') {
        Some("") => home(),
        Some(rest) if rest.starts_with(separator) => home().join(&rest[1..]),
        _ => PathBuf::from(path),
    }
}

/// [`std::fs::canonicalize`] without the `\\?\` prefix Windows puts on drive paths, which PATH
/// entries, prompts and other programs do not take.
pub fn canonicalize(path: &Path) -> std::io::Result<PathBuf> {
    let path = std::fs::canonicalize(path)?;
    #[cfg(windows)]
    if let Some(rest) = path.to_str().and_then(|p| p.strip_prefix(r"\\?\"))
        && rest.as_bytes().get(1) == Some(&b':')
    {
        return Ok(PathBuf::from(rest));
    }
    Ok(path)
}

pub fn data() -> PathBuf {
    root().join("data")
}

pub fn registry() -> PathBuf {
    data().join("registry.json")
}

pub fn config_file() -> PathBuf {
    data().join("config.json")
}

pub fn profile() -> PathBuf {
    data().join("profile.md")
}

/// The submission ledger: every contest already submitted (git-ignored).
pub fn submissions() -> PathBuf {
    data().join("submissions.json")
}

pub fn board() -> PathBuf {
    root().join("BOARD.md")
}

pub fn prompts() -> PathBuf {
    root().join("harness").join("prompts")
}

pub fn templates() -> PathBuf {
    root().join("harness").join("templates")
}

pub fn agents() -> PathBuf {
    root().join("agents")
}

pub fn skill() -> PathBuf {
    root().join("skills").join("contest-harness")
}

/// A configured file path: `~`/absolute as given, anything else relative to the repository root.
pub fn resolve(path: &str) -> PathBuf {
    let expanded = expand(path);
    if expanded.is_absolute() { expanded } else { root().join(expanded) }
}

pub fn omp_agent_dir() -> PathBuf {
    home().join(".omp").join("agent")
}
