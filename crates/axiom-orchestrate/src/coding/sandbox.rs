//! Fail-closed workspace sandbox for the coding agent. Every path is canonicalized
//! (which RESOLVES symlinks) and asserted to be a descendant of the root, so `..` and
//! symlink-out escapes fail closed. Commands run WITHOUT a shell (argv vector, no string
//! interpolation) against a READ-ONLY allowlist, with a hard timeout + child kill on
//! drop, and all output is byte-capped.
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_READ: usize = 256 * 1024;
const MAX_OUT: usize = 64 * 1024;
const MAX_WRITE: usize = 2 * 1024 * 1024;
/// Path components never exposed/written, even inside the root (secrets / vcs internals).
const DENY: &[&str] = &[".git", ".env", ".ssh", ".aws", "id_rsa", "id_ed25519", ".npmrc"];

/// Read-only (production default) vs read-write (an opted-in coding task / eval scratch).
/// Writes and build/test execution are refused entirely in `ReadOnly`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxMode {
    ReadOnly,
    ReadWrite,
}

pub struct Sandbox {
    root: PathBuf,
    mode: SandboxMode,
}

#[derive(Debug)]
pub struct CmdResult {
    pub code: Option<i32>,
    pub output: String,
}

impl Sandbox {
    /// Read-only sandbox (the production default).
    pub fn new(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        Self::with_mode(root, SandboxMode::ReadOnly)
    }

    pub fn with_mode(root: impl AsRef<Path>, mode: SandboxMode) -> anyhow::Result<Self> {
        let root = std::fs::canonicalize(root.as_ref())
            .map_err(|e| anyhow::anyhow!("workspace not found: {e}"))?;
        if !root.is_dir() {
            anyhow::bail!("workspace is not a directory");
        }
        Ok(Self { root, mode })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn mode(&self) -> SandboxMode {
        self.mode
    }

    /// Resolve a (relative or absolute) path to a real path strictly inside the root, or
    /// reject. `canonicalize` follows symlinks, so a symlink inside the root pointing out
    /// resolves to an outside path and fails the prefix check.
    fn resolve(&self, p: &str) -> Result<PathBuf, String> {
        let raw = Path::new(p);
        for c in raw.components() {
            if let std::path::Component::Normal(os) = c {
                if DENY.iter().any(|d| os == *d) {
                    return Err(format!("access denied: {p}"));
                }
            }
        }
        let joined = if raw.is_absolute() { raw.to_path_buf() } else { self.root.join(raw) };
        let canon = std::fs::canonicalize(&joined).map_err(|_| format!("no such path: {p}"))?;
        if !canon.starts_with(&self.root) {
            return Err(format!("path escapes the workspace: {p}"));
        }
        Ok(canon)
    }

    pub fn read_file(&self, p: &str) -> Result<String, String> {
        let path = self.resolve(p)?;
        if !path.is_file() {
            return Err(format!("not a file: {p}"));
        }
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        let mut s = String::from_utf8_lossy(&bytes).into_owned();
        cap_head(&mut s, MAX_READ);
        Ok(s)
    }

    pub fn list_dir(&self, p: &str) -> Result<String, String> {
        let path = self.resolve(p)?;
        if !path.is_dir() {
            return Err(format!("not a directory: {p}"));
        }
        let mut entries: Vec<String> = std::fs::read_dir(&path)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok())
            .filter(|e| !DENY.iter().any(|d| e.file_name() == *std::ffi::OsStr::new(d)))
            .map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if e.path().is_dir() {
                    format!("{name}/")
                } else {
                    name
                }
            })
            .collect();
        entries.sort();
        entries.truncate(400);
        Ok(entries.join("\n"))
    }

    /// Resolve a path for WRITING: refused in read-only mode; rejects `..`, DENY
    /// components, and any target whose existing parent escapes the root (symlink-out
    /// included, since the parent is canonicalized). The file itself need not exist yet.
    fn resolve_for_write(&self, p: &str) -> Result<PathBuf, String> {
        if self.mode != SandboxMode::ReadWrite {
            return Err("workspace is read-only".into());
        }
        let raw = Path::new(p);
        for c in raw.components() {
            match c {
                std::path::Component::ParentDir => return Err(format!("'..' not allowed: {p}")),
                std::path::Component::Normal(os) if DENY.iter().any(|d| os == *d) => {
                    return Err(format!("write denied: {p}"))
                }
                _ => {}
            }
        }
        let joined = if raw.is_absolute() { raw.to_path_buf() } else { self.root.join(raw) };
        let parent = joined.parent().ok_or_else(|| format!("no parent dir: {p}"))?;
        let cparent = std::fs::canonicalize(parent).map_err(|_| format!("parent dir does not exist: {p}"))?;
        if !cparent.starts_with(&self.root) {
            return Err(format!("path escapes the workspace: {p}"));
        }
        let fname = joined.file_name().ok_or_else(|| format!("no file name: {p}"))?;
        Ok(cparent.join(fname))
    }

    /// Create or overwrite a file (read-write mode only). Atomic temp-write + rename so a
    /// crash never leaves a half-written file.
    pub fn write_file(&self, p: &str, content: &str) -> Result<(), String> {
        let path = self.resolve_for_write(p)?;
        if content.len() > MAX_WRITE {
            return Err(format!("content too large ({} bytes, max {})", content.len(), MAX_WRITE));
        }
        let tmp = path.with_file_name(format!(
            ".{}.axiomtmp{}",
            path.file_name().and_then(|s| s.to_str()).unwrap_or("f"),
            std::process::id()
        ));
        std::fs::write(&tmp, content).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            e.to_string()
        })?;
        Ok(())
    }

    /// Run an allowlisted, shell-free command in the workspace root with a hard timeout.
    /// The allowlist widens to build/test tools in read-write mode.
    pub async fn run_command(&self, argv: &[String], timeout: Duration) -> Result<CmdResult, String> {
        let prog = argv.first().ok_or("empty command")?;
        if !cmd_allowed(prog, argv, self.mode) {
            return Err(format!("command not allowed: {}", argv.join(" ")));
        }
        use tokio::process::Command;
        let mut cmd = Command::new(prog);
        cmd.args(&argv[1..])
            .current_dir(&self.root)
            .kill_on_drop(true)
            .stdin(std::process::Stdio::null());
        let out = match tokio::time::timeout(timeout, cmd.output()).await {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => return Err(format!("spawn failed: {e}")),
            Err(_) => return Err(format!("command timed out after {}s", timeout.as_secs())),
        };
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        if !out.stderr.is_empty() {
            text.push_str("\n[stderr]\n");
            text.push_str(&String::from_utf8_lossy(&out.stderr));
        }
        cap_tail(&mut text, MAX_OUT);
        Ok(CmdResult { code: out.status.code(), output: text })
    }
}

/// Command allowlist. Inspect/search + read-only `git` are always allowed; build/test
/// runners (which execute project code) are allowed ONLY in read-write mode. No shell
/// ever; `git push`/`rm`/`curl`/network fetchers are always refused.
fn cmd_allowed(prog: &str, argv: &[String], mode: SandboxMode) -> bool {
    const READ: &[&str] = &["ls", "cat", "rg", "grep", "find", "wc", "head", "tail", "tree"];
    const BUILD: &[&str] = &[
        "cargo", "rustc", "npm", "pnpm", "yarn", "bun", "node", "deno", "tsc",
        "python", "python3", "pytest", "go", "make", "jest", "mocha", "vitest",
    ];
    const GIT_RO: &[&str] = &["status", "log", "diff", "show", "ls-files", "branch", "rev-parse", "describe", "blame"];
    let base = Path::new(prog).file_name().and_then(|s| s.to_str()).unwrap_or(prog);
    if base == "git" {
        // git is always read-only (no push/remote-mutating subcommands), even in RW mode.
        return argv.get(1).map(|s| GIT_RO.contains(&s.as_str())).unwrap_or(false);
    }
    if READ.contains(&base) {
        return true;
    }
    mode == SandboxMode::ReadWrite && BUILD.contains(&base)
}

/// Keep the first `max` bytes (on a char boundary); note truncation.
fn cap_head(s: &mut String, max: usize) {
    if s.len() <= max {
        return;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    s.push_str("\n…[truncated]");
}

/// Keep the LAST `max` bytes (errors usually trail); note truncation.
fn cap_tail(s: &mut String, max: usize) {
    if s.len() <= max {
        return;
    }
    let mut start = s.len() - max;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    let tail = s[start..].to_string();
    *s = format!("…[truncated]\n{tail}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(label: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("axiom-sbx-{}-{}", label, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn reads_inside_but_rejects_escape() {
        let root = tmp("esc");
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        let sb = Sandbox::new(&root).unwrap();
        assert_eq!(sb.read_file("a.txt").unwrap(), "hello");
        assert!(sb.read_file("../../../etc/passwd").is_err());
        assert!(sb.read_file("/etc/passwd").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn denies_dotgit_and_secrets() {
        let root = tmp("deny");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git").join("config"), "x").unwrap();
        std::fs::write(root.join(".env"), "SECRET=1").unwrap();
        let sb = Sandbox::new(&root).unwrap();
        assert!(sb.read_file(".git/config").is_err());
        assert!(sb.read_file(".env").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_out_of_root() {
        let root = tmp("sym");
        let outside = tmp("sym-outside");
        std::fs::write(outside.join("secret.txt"), "top secret").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("link.txt")).unwrap();
        let sb = Sandbox::new(&root).unwrap();
        // the symlink resolves (canonicalize) to outside → rejected
        assert!(sb.read_file("link.txt").is_err());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn allowlist_blocks_dangerous_commands() {
        use SandboxMode::*;
        // read tools allowed in either mode
        assert!(cmd_allowed("rg", &["rg".into(), "foo".into()], ReadOnly));
        assert!(cmd_allowed("/usr/bin/grep", &["grep".into(), "x".into()], ReadOnly));
        assert!(cmd_allowed("git", &["git".into(), "status".into()], ReadOnly));
        // git push refused even in RW
        assert!(!cmd_allowed("git", &["git".into(), "push".into()], ReadWrite));
        // dangerous always refused
        assert!(!cmd_allowed("rm", &["rm".into(), "-rf".into(), "/".into()], ReadWrite));
        assert!(!cmd_allowed("curl", &["curl".into(), "evil".into()], ReadWrite));
        assert!(!cmd_allowed("sh", &["sh".into(), "-c".into(), "x".into()], ReadWrite));
        // build/test runners: refused in ReadOnly, allowed in ReadWrite
        assert!(!cmd_allowed("cargo", &["cargo".into(), "test".into()], ReadOnly));
        assert!(cmd_allowed("cargo", &["cargo".into(), "test".into()], ReadWrite));
        assert!(cmd_allowed("pytest", &["pytest".into()], ReadWrite));
        assert!(!cmd_allowed("pytest", &["pytest".into()], ReadOnly));
    }

    #[test]
    fn writes_only_in_rw_and_stay_inside_root() {
        let root = tmp("wr");
        // read-only sandbox refuses writes
        let ro = Sandbox::new(&root).unwrap();
        assert!(ro.write_file("a.txt", "hi").is_err());
        // read-write sandbox writes inside root
        let rw = Sandbox::with_mode(&root, SandboxMode::ReadWrite).unwrap();
        assert!(rw.write_file("a.txt", "hello").is_ok());
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "hello");
        assert_eq!(rw.read_file("a.txt").unwrap(), "hello");
        // escapes + denied paths refused even in RW
        assert!(rw.write_file("../escape.txt", "x").is_err());
        assert!(rw.write_file("/tmp/escape.txt", "x").is_err());
        assert!(rw.write_file(".git/config", "x").is_err());
        assert!(rw.write_file(".env", "x").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn output_caps_dont_split_utf8() {
        let mut s = "✓".repeat(100_000);
        cap_tail(&mut s, 64 * 1024);
        assert!(s.is_char_boundary(s.len()));
        let mut h = "✓".repeat(100_000);
        cap_head(&mut h, 64 * 1024);
        assert!(h.is_char_boundary(h.len()));
    }
}
