//! the places where macOS, Linux and Windows differ: the data directory, how a path is spelt,
//! file identity, whether a process is alive and how a pre-image copy is made.
//!
//! everything else in the crate goes through these functions, so a platform difference is
//! handled once rather than at every call site.

use anyhow::{Context, Result};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf, Prefix};

/// the data directory is `FOOTPRINTS_HOME` when set. Otherwise it is `~/.local/share/footprints`
/// on macOS and Linux, the same path on both so it is easy to find, and
/// `%LOCALAPPDATA%\footprints` on Windows, which sits inside the user profile where only that
/// account, SYSTEM and administrators can read it.
pub fn default_home() -> Result<PathBuf> {
    if let Some(h) = std::env::var_os("FOOTPRINTS_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(h));
    }
    #[cfg(windows)]
    {
        let base = std::env::var_os("LOCALAPPDATA")
            .filter(|v| !v.is_empty())
            .context("LOCALAPPDATA is not set and FOOTPRINTS_HOME is empty")?;
        Ok(PathBuf::from(base).join("footprints"))
    }
    #[cfg(not(windows))]
    {
        let home =
            std::env::var_os("HOME").context("HOME is not set and FOOTPRINTS_HOME is empty")?;
        Ok(PathBuf::from(home).join(".local/share/footprints"))
    }
}

/// the user's home directory, used only to shorten paths for display.
pub fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// `canonicalize` without the `\\?\` prefix Windows adds. The ledger stores paths the way
/// agents and people write them (`C:\Users\me\f.txt`), so a canonical path with the verbatim
/// prefix would never match the same file named in a hook payload.
pub fn canonical(path: &Path) -> io::Result<PathBuf> {
    let c = fs::canonicalize(path)?;
    Ok(strip_verbatim(&c))
}

/// turns `\\?\C:\x` into `C:\x` and `\\?\UNC\server\share\x` into `\\server\share\x`. Other
/// verbatim forms (device paths, volume GUIDs) are kept, since they have no plain spelling.
pub fn strip_verbatim(path: &Path) -> PathBuf {
    let mut comps = path.components();
    let Some(Component::Prefix(p)) = comps.next() else {
        return path.to_path_buf();
    };
    let rest: PathBuf = comps.collect();
    match p.kind() {
        Prefix::VerbatimDisk(d) => {
            let mut out = PathBuf::from(format!("{}:\\", (d as char).to_ascii_uppercase()));
            for c in rest
                .components()
                .filter(|c| !matches!(c, Component::RootDir))
            {
                out.push(c);
            }
            out
        }
        Prefix::VerbatimUNC(server, share) => {
            let mut out = PathBuf::from(format!(
                "\\\\{}\\{}\\",
                server.to_string_lossy(),
                share.to_string_lossy()
            ));
            for c in rest
                .components()
                .filter(|c| !matches!(c, Component::RootDir))
            {
                out.push(c);
            }
            out
        }
        _ => path.to_path_buf(),
    }
}

/// removes `.` and resolves `..` without touching the disk, so `sub/../../x` is stored as the
/// path it really names and two spellings of one file match. On Windows forward slashes become
/// backslashes, a drive letter is upper-cased and a verbatim prefix is dropped, so
/// `c:/work/a.txt`, `C:\work\.\a.txt` and `\\?\C:\work\a.txt` are the same entry.
pub fn normalise(path: &Path) -> PathBuf {
    let path = strip_verbatim(path);
    let mut out = PathBuf::new();
    let mut rooted = false;
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                // `..` at the root names the root itself, as the kernel resolves it
                if !out.pop() && !rooted {
                    out.push("..");
                } else if out.as_os_str().is_empty() && rooted {
                    out.push(Component::RootDir.as_os_str());
                }
            }
            Component::RootDir => {
                rooted = true;
                out.push(c.as_os_str());
            }
            Component::Prefix(p) => match p.kind() {
                Prefix::Disk(d) => out.push(format!("{}:", (d as char).to_ascii_uppercase())),
                _ => out.push(c.as_os_str()),
            },
            Component::Normal(n) => out.push(trim_windows_name(n)),
        }
    }
    out
}

/// Windows drops trailing dots and spaces from a file name, so `a.txt.` and `a.txt ` open the
/// same file as `a.txt`. A name made only of them is kept, since it cannot be shortened.
#[cfg(windows)]
fn trim_windows_name(n: &std::ffi::OsStr) -> std::ffi::OsString {
    match n.to_str() {
        Some(s) => {
            let t = s.trim_end_matches(['.', ' ']);
            if t.is_empty() {
                n.to_os_string()
            } else {
                t.into()
            }
        }
        None => n.to_os_string(),
    }
}

#[cfg(not(windows))]
fn trim_windows_name(n: &std::ffi::OsStr) -> std::ffi::OsString {
    n.to_os_string()
}

/// the form two paths are compared in. Windows file systems ignore case, so the key is
/// lower-cased there; the path itself is always stored as written.
pub fn path_key(p: &Path) -> String {
    let s = normalise(p).to_string_lossy().into_owned();
    if cfg!(windows) { s.to_lowercase() } else { s }
}

/// true when two paths name the same file by their comparison keys.
pub fn same_key(a: &Path, b: &Path) -> bool {
    path_key(a) == path_key(b)
}

/// `Path::starts_with` by comparison keys, so `C:\Work\a` is under `c:\work` on Windows.
pub fn key_starts_with(p: &Path, root: &Path) -> bool {
    if cfg!(windows) {
        let lower = |x: &Path| PathBuf::from(normalise(x).to_string_lossy().to_lowercase());
        lower(p).starts_with(lower(root))
    } else {
        p.starts_with(root)
    }
}

/// a path with its parent directory resolved through any symlinks, as the file system names
/// it. Two spellings of one file from different agents then become one ledger entry. When the
/// parent does not exist yet the normalised path is returned.
pub fn resolve_parent(p: &Path) -> PathBuf {
    let p = normalise(p);
    match (p.parent(), p.file_name()) {
        (Some(dir), Some(name)) if !dir.as_os_str().is_empty() => match canonical(dir) {
            Ok(d) => d.join(name),
            Err(_) => p,
        },
        _ => p,
    }
}

/// a pair that changes whenever the file's contents could have changed, beyond its size and
/// mtime: the inode and change time on Unix, the file index and change time on Windows. A
/// rewrite can keep the size and put the old mtime back (`touch -r`, `cp -p`), but the change
/// time moves forward. `(0, 0)` means neither could be read. Callers then never trust a
/// cached hash for the file.
#[cfg(unix)]
pub fn file_identity(_path: &Path, md: &fs::Metadata) -> (u64, i64) {
    use std::os::unix::fs::MetadataExt;
    (
        md.ino(),
        md.ctime().saturating_mul(1_000_000_000) + md.ctime_nsec(),
    )
}

#[cfg(windows)]
pub fn file_identity(path: &Path, _md: &fs::Metadata) -> (u64, i64) {
    win::file_identity(path).unwrap_or((0, 0))
}

#[cfg(not(any(unix, windows)))]
pub fn file_identity(_path: &Path, _md: &fs::Metadata) -> (u64, i64) {
    (0, 0)
}

/// true while a process with this id exists. Used to tell a running wrapper from one that was
/// killed and left its staging area behind.
#[cfg(unix)]
pub fn process_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 only checks that the process exists and sends nothing
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(windows)]
pub fn process_alive(pid: u32) -> bool {
    win::process_alive(pid)
}

#[cfg(not(any(unix, windows)))]
pub fn process_alive(_pid: u32) -> bool {
    true
}

/// from `sys/clonefile.h`; the libc crate does not export it.
#[cfg(target_os = "macos")]
const CLONE_NOFOLLOW: u32 = 0x0001;

/// copies a file for a pre-image as cheaply as the file system allows. APFS `clonefile` on
/// macOS and a `FICLONE` reflink on Linux (Btrfs, XFS, bcachefs) share blocks with the original,
/// so the copy costs almost nothing until the file changes. Everything else, Windows included,
/// gets a plain copy.
#[cfg(target_os = "macos")]
pub fn clone_or_copy(src: &Path, dst: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let s = CString::new(src.as_os_str().as_bytes())?;
    let d = CString::new(dst.as_os_str().as_bytes())?;
    // SAFETY: both pointers are valid NUL-terminated strings that outlive the call
    let rc = unsafe { libc::clonefile(s.as_ptr(), d.as_ptr(), CLONE_NOFOLLOW) };
    if rc == 0 {
        return Ok(());
    }
    // clonefile fails across volumes and on non-APFS disks, where a real copy still works
    fs::copy(src, dst).map(|_| ())
}

#[cfg(target_os = "linux")]
pub fn clone_or_copy(src: &Path, dst: &Path) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let from = fs::File::open(src)?;
    let to = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dst)?;
    // SAFETY: both descriptors are valid for the duration of the call
    let rc = unsafe { libc::ioctl(to.as_raw_fd(), libc::FICLONE, from.as_raw_fd()) };
    if rc == 0 {
        if let Ok(md) = from.metadata() {
            let _ = to.set_permissions(md.permissions());
        }
        return Ok(());
    }
    // ext4, tmpfs and copies across file systems cannot share blocks
    drop(to);
    fs::remove_file(dst)?;
    fs::copy(src, dst).map(|_| ())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn clone_or_copy(src: &Path, dst: &Path) -> io::Result<()> {
    // ReFS block cloning needs cluster-aligned extents and a pre-sized sparse target, which is
    // more machinery than a pre-image is worth for now, so Windows always copies
    fs::copy(src, dst).map(|_| ())
}

/// replaces `to` with `from` in one step. std's rename is `rename(2)` on Unix and
/// `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING` (or the POSIX-semantics rename where the
/// file system supports it) on Windows, so the target is never half-written. Windows refuses
/// when another process holds the target open without delete sharing. That error is
/// returned as is.
pub fn replace(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

/// the name of a running process, for notes from the whole-machine daemon.
#[cfg(target_os = "linux")]
pub fn process_name(pid: u32) -> Option<String> {
    fs::read_to_string(format!("/proc/{pid}/comm"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(windows)]
pub(crate) mod win {
    //! the few Win32 calls std does not wrap.

    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, GetLastError, HANDLE, STILL_ACTIVE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FileBasicInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    pub fn file_identity(path: &Path) -> std::io::Result<(u64, i64)> {
        // attribute access with every share flag, so reading the identity never gets in the
        // way of another program that has the file open
        let f = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        let h = f.as_raw_handle() as HANDLE;
        // SAFETY: zeroed plain-data structs are valid and both calls write at most their size
        unsafe {
            let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
            if GetFileInformationByHandle(h, &mut info) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            let mut basic: FILE_BASIC_INFO = std::mem::zeroed();
            if GetFileInformationByHandleEx(
                h,
                FileBasicInfo,
                (&raw mut basic).cast(),
                std::mem::size_of::<FILE_BASIC_INFO>() as u32,
            ) == 0
            {
                return Err(std::io::Error::last_os_error());
            }
            let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
            // ChangeTime counts 100 ns ticks; it is only ever compared for equality
            Ok((index, basic.ChangeTime))
        }
    }

    pub fn process_alive(pid: u32) -> bool {
        // SAFETY: the handle is checked before use and closed exactly once
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                // a process that exists but belongs to someone else cannot be opened
                return GetLastError() == ERROR_ACCESS_DENIED;
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(h, &mut code);
            CloseHandle(h);
            ok != 0 && code == STILL_ACTIVE as u32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_resolves_dot_dot() {
        let p = |s: &str| PathBuf::from(s);
        if cfg!(windows) {
            assert_eq!(
                normalise(Path::new(r"C:\a\sub\..\..\escape.txt")),
                p(r"C:\escape.txt")
            );
            assert_eq!(normalise(Path::new(r"C:\a\.\b")), p(r"C:\a\b"));
        } else {
            assert_eq!(
                normalise(Path::new("/a/sub/../../escape.txt")),
                p("/escape.txt")
            );
            assert_eq!(normalise(Path::new("/a/./b")), p("/a/b"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn dot_dot_at_the_root_stays_at_the_root() {
        assert_eq!(normalise(Path::new("/../x")), PathBuf::from("/x"));
        assert_eq!(normalise(Path::new("a/../../x")), PathBuf::from("../x"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_spellings_of_one_path_agree() {
        let want = PathBuf::from(r"C:\work\a.txt");
        for s in [
            r"C:\work\a.txt",
            "c:/work/a.txt",
            r"c:\work\.\sub\..\a.txt",
            r"\\?\C:\work\a.txt",
            r"C:\..\work\a.txt",
        ] {
            assert_eq!(normalise(Path::new(s)), want, "{s}");
        }
        assert_eq!(
            normalise(Path::new(r"\\?\UNC\server\share\x\..\y.txt")),
            PathBuf::from(r"\\server\share\y.txt")
        );
        assert_eq!(
            normalise(Path::new("//server/share/d/e.txt")),
            PathBuf::from(r"\\server\share\d\e.txt")
        );
    }

    #[cfg(windows)]
    #[test]
    fn trailing_dots_and_spaces_and_case_are_one_key() {
        assert_eq!(
            normalise(Path::new(r"C:\work\a.txt. ")),
            PathBuf::from(r"C:\work\a.txt")
        );
        assert!(same_key(
            Path::new(r"c:\Work\A.TXT"),
            Path::new(r"C:\work\a.txt.")
        ));
        assert!(key_starts_with(
            Path::new(r"C:\WORK\sub\f"),
            Path::new(r"c:\work")
        ));
        // the stored spelling is kept
        assert_eq!(
            normalise(Path::new(r"C:\Work\A.TXT")),
            PathBuf::from(r"C:\Work\A.TXT")
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_keys_keep_case() {
        assert!(!same_key(Path::new("/a/B"), Path::new("/a/b")));
        assert!(!key_starts_with(Path::new("/A/b"), Path::new("/a")));
    }

    #[test]
    fn resolve_parent_follows_a_symlinked_directory() {
        let d = tempfile::tempdir().unwrap();
        let real = canonical(d.path()).unwrap();
        let p = resolve_parent(&d.path().join("f.txt"));
        assert_eq!(p, real.join("f.txt"));
        let missing = d.path().join("no/such/f.txt");
        assert_eq!(resolve_parent(&missing), normalise(&missing));
    }

    #[cfg(windows)]
    #[test]
    fn canonical_has_no_verbatim_prefix() {
        let d = tempfile::tempdir().unwrap();
        let c = canonical(d.path()).unwrap();
        assert!(!c.to_string_lossy().starts_with(r"\\?\"), "{}", c.display());
        assert!(c.is_absolute());
    }

    #[cfg(windows)]
    #[test]
    fn file_identity_changes_on_rewrite() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f");
        fs::write(&f, "one").unwrap();
        let before = win::file_identity(&f).unwrap();
        assert_ne!(before.0, 0);
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&f, "two").unwrap();
        let after = win::file_identity(&f).unwrap();
        assert_eq!(before.0, after.0, "same file, same index");
        assert_ne!(before.1, after.1, "a write moves the change time");
    }

    #[test]
    fn this_process_is_alive() {
        assert!(process_alive(std::process::id()));
    }

    #[test]
    fn default_home_honours_the_override() {
        // read-only check: the variable is set by the test harness for child processes only,
        // so this asserts the shape of the fallback instead of mutating the environment
        if std::env::var_os("FOOTPRINTS_HOME").is_none() {
            let h = default_home().unwrap();
            assert!(h.ends_with("footprints"), "{}", h.display());
        }
    }
}
