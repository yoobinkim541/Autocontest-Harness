//! What differs between operating systems. Unix has process groups, signals and file modes; Windows
//! has none of those, so a session's process tree is stopped with `taskkill /T`, memory and process
//! start times come from kernel32, and private files rely on the user profile's ACL.

use std::fs::{DirBuilder, Metadata, OpenOptions};
use std::path::Path;
use std::process::Command;

/// The permission bits of a file or folder (`mode & 0o777`); `None` where the OS has no modes.
pub fn mode(meta: &Metadata) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(meta.permissions().mode() & 0o777)
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        None
    }
}

/// Set the permission bits of `path`; nothing to do where the OS has no modes.
pub fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Ok(())
    }
}

/// A recursive folder builder that creates with `mode` where the OS has modes.
pub fn dir_builder(mode: u32) -> DirBuilder {
    let mut builder = DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    builder
}

/// `options` that create new files with `mode` where the OS has modes.
pub fn with_mode(options: &mut OpenOptions, mode: u32) -> &mut OpenOptions {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    options
}

/// Run `command` in a process group of its own, without a console window on Windows, so that a
/// timeout can stop it together with its children ([`kill_tree`]).
pub fn own_group(command: &mut Command) -> &mut Command {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(win::CREATE_NEW_PROCESS_GROUP | win::CREATE_NO_WINDOW)
    }
}

/// Kill the process group led by `pid` (Unix) or the process tree under `pid` (Windows).
pub fn kill_tree(pid: u32) {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }
        const SIGKILL: i32 = 9;
        if let Ok(pgid) = i32::try_from(pid)
            && pgid > 0
        {
            // SAFETY: plain syscall; a negative pid addresses the group the caller created.
            unsafe {
                kill(-pgid, SIGKILL);
            }
        }
    }
    #[cfg(windows)]
    win::taskkill(pid);
}

#[cfg(windows)]
pub use win::{ignore_console_interrupts, memory_status, process_start, taskkill};

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::process::{Command, Stdio};

    pub const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    const CTRL_C_EVENT: u32 = 0;
    const CTRL_BREAK_EVENT: u32 = 1;

    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }

    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(buffer: *mut MemoryStatusEx) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn GetExitCodeProcess(process: *mut c_void, code: *mut u32) -> i32;
        fn GetProcessTimes(
            process: *mut c_void,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn SetConsoleCtrlHandler(handler: Option<unsafe extern "system" fn(u32) -> i32>, add: i32) -> i32;
    }

    /// Available physical memory in bytes and the share of physical memory in use (0-100).
    pub fn memory_status() -> Option<(u64, u32)> {
        let mut status = MemoryStatusEx {
            length: size_of::<MemoryStatusEx>() as u32,
            memory_load: 0,
            total_phys: 0,
            avail_phys: 0,
            total_page_file: 0,
            avail_page_file: 0,
            total_virtual: 0,
            avail_virtual: 0,
            avail_extended_virtual: 0,
        };
        // SAFETY: `status` is a correctly sized MEMORYSTATUSEX with `dwLength` set, as the API requires.
        (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then_some((status.avail_phys, status.memory_load))
    }

    /// The creation time of a running process `pid` (100 ns ticks since 1601, as text); `None`
    /// when no such process runs.
    pub fn process_start(pid: u32) -> Option<String> {
        // SAFETY: OpenProcess returns null on failure; the handle is closed below on every path.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return None;
        }
        let (mut code, mut created) = (0u32, FileTime::default());
        let (mut exited, mut kernel, mut user) = (FileTime::default(), FileTime::default(), FileTime::default());
        // SAFETY: `handle` is a live process handle with query rights; the out pointers are valid.
        let alive = unsafe { GetExitCodeProcess(handle, &mut code) } != 0 && code == STILL_ACTIVE;
        let timed = unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) } != 0;
        // SAFETY: closes the handle opened above exactly once.
        unsafe { CloseHandle(handle) };
        (alive && timed).then(|| ((u64::from(created.high) << 32) | u64::from(created.low)).to_string())
    }

    /// `taskkill /T /F /PID <pid>`: the process and every process it started. Whether it worked.
    pub fn taskkill(pid: u32) -> bool {
        use std::os::windows::process::CommandExt;
        Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .is_ok_and(|s| s.success())
    }

    unsafe extern "system" fn swallow(event: u32) -> i32 {
        i32::from(event == CTRL_C_EVENT || event == CTRL_BREAK_EVENT)
    }

    /// Ctrl+C and Ctrl+Break reach every process on the console. A parent that only waits for a
    /// child on the same console ignores them, so the child alone decides what they mean. The
    /// handler is not inherited, so the child keeps the default behavior.
    pub fn ignore_console_interrupts() {
        // SAFETY: registers a handler with the signature the API expects; it touches no state.
        unsafe { SetConsoleCtrlHandler(Some(swallow), 1) };
    }
}
