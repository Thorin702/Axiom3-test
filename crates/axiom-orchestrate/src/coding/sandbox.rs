//! Fail-closed workspace sandbox for the coding agent. Every path — including path-like
//! command arguments — is canonicalized (which RESOLVES symlinks) and asserted to be a
//! descendant of the root, with DENY re-checked on the resolved path so a symlink can't
//! hide its target. Commands run WITHOUT a shell (argv vector, no string interpolation)
//! against a READ-ONLY allowlist, with a hard timeout + child kill on drop, and all output
//! is byte-capped.
//!
//! SCOPE: the read-only path is a real jail. The read-WRITE path additionally runs BUILD
//! tools (cargo/npm/node/python/make/…) which execute arbitrary project code; `scrub_env`
//! strips credentials and forces offline defaults, but ReadWrite is NOT a hardened security
//! boundary and must only be used on trusted workspaces.
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

    /// True if the CANONICAL path traverses a DENY-named component *inside the root*. The
    /// raw-string DENY check is defeated by a symlink whose name hides its target (e.g.
    /// `g -> .git`, so `g/config` reads `.git/config`); re-checking the resolved path closes
    /// that. Only components below the root are inspected, so a root that itself lives under
    /// e.g. `.aws/` is not self-denied.
    fn deny_hit(&self, canon: &Path) -> bool {
        match canon.strip_prefix(&self.root) {
            Ok(rel) => rel.components().any(|c| {
                matches!(c, std::path::Component::Normal(os) if DENY.iter().any(|d| os == *d))
            }),
            Err(_) => true, // outside the root → treat as denied
        }
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
        if self.deny_hit(&canon) {
            return Err(format!("access denied: {p}"));
        }
        Ok(canon)
    }

    pub fn read_file(&self, p: &str) -> Result<String, String> {
        let path = self.resolve(p)?;
        if !path.is_file() {
            return Err(format!("not a file: {p}"));
        }
        // Read AT MOST the cap (+a little) into memory — never `fs::read` the whole file, or a
        // multi-GB workspace file would OOM the host before cap_head ever ran.
        let bytes = read_capped(&path, MAX_READ).map_err(|e| e.to_string())?;
        let mut s = String::from_utf8_lossy(&bytes).into_owned();
        cap_head(&mut s, MAX_READ);
        Ok(s)
    }

    /// Uncapped read for EDITING (up to MAX_WRITE) — editing must round-trip the WHOLE file, so it
    /// can't go through the model-facing capped `read_file` (that truncated large files and then
    /// wrote the truncation back, silently destroying everything past the cap). A file too large to
    /// write back is refused rather than corrupted.
    fn read_file_full(&self, p: &str) -> Result<String, String> {
        let path = self.resolve(p)?;
        if !path.is_file() {
            return Err(format!("not a file: {p}"));
        }
        let len = std::fs::metadata(&path).map_err(|e| e.to_string())?.len() as usize;
        if len > MAX_WRITE {
            return Err(format!(
                "edit: {p} is {} KB — too large to edit safely (limit {} KB). Rewrite the relevant part with write_file, or operate on it with the python tool.",
                len / 1024, MAX_WRITE / 1024
            ));
        }
        std::fs::read_to_string(&path).map_err(|e| e.to_string())
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
        let target = cparent.join(fname);
        // Re-check DENY on the resolved parent: a symlinked dir component (e.g. `g -> .git`)
        // canonicalizes back inside the root but must still not let a write land in `.git`.
        if self.deny_hit(&target) {
            return Err(format!("write denied: {p}"));
        }
        Ok(target)
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

    /// Precisely edit a file (read-write mode): replace the UNIQUE occurrence of `old` with
    /// `new`. Refused — not misapplied — if `old` is absent or appears more than once, so the
    /// agent must include enough surrounding context to target exactly one spot. This is the
    /// key upgrade over rewriting the whole file: it scales to large files and can't silently
    /// clobber. Inherits read/write permission + atomic write from read_file/write_file.
    pub fn edit_file(&self, p: &str, old: &str, new: &str) -> Result<(), String> {
        if old.is_empty() {
            return Err("edit: 'old' must not be empty".into());
        }
        if old == new {
            return Err("edit: 'old' and 'new' are identical (nothing to do)".into());
        }
        let content = self.read_file_full(p)?; // uncapped — must not truncate the file on write-back
        let count = content.matches(old).count();
        if count == 0 {
            return Err(format!("edit: 'old' text not found in {p}"));
        }
        if count > 1 {
            return Err(format!(
                "edit: 'old' matches {count} places in {p}; include more surrounding context so it's unique"
            ));
        }
        let updated = content.replacen(old, new, 1);
        self.write_file(p, &updated)
    }

    /// Find files by a simplified glob within the workspace — PURE RUST, no external `rg`
    /// (which may be absent). Supports `**/`, `*.ext`, `*substr*`, `prefix*`, `*suffix`, and
    /// plain names; skips .git / target / node_modules / dotdirs. Up to 200 relative paths.
    pub fn glob(&self, pattern: &str) -> Result<String, String> {
        let pat = pattern.trim().trim_start_matches("./").trim_start_matches("**/");
        let mut out: Vec<String> = Vec::new();
        let mut stack = vec![self.root.clone()];
        let mut budget: i64 = 50_000;
        while let Some(dir) = stack.pop() {
            let rd = match std::fs::read_dir(&dir) {
                Ok(r) => r,
                Err(_) => continue,
            };
            for e in rd.flatten() {
                budget -= 1;
                if budget < 0 {
                    break;
                }
                let name = e.file_name().to_string_lossy().into_owned();
                let path = e.path();
                if path.is_dir() {
                    if name == ".git" || name == "target" || name == "node_modules" || name.starts_with('.') {
                        continue;
                    }
                    stack.push(path);
                    continue;
                }
                if glob_match(pat, &name) {
                    if let Ok(rel) = path.strip_prefix(&self.root) {
                        out.push(rel.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
        }
        out.sort();
        out.truncate(200);
        Ok(if out.is_empty() { "(no matching files)".into() } else { out.join("\n") })
    }

    /// Reject command arguments that point outside the workspace or at DENY paths, so the
    /// allowlisted read tools (`cat`/`grep`/`find`/…) can't be turned into a filesystem
    /// exfiltrator (`cat /etc/passwd`, `cat .env`, `find / -name id_rsa`). Flags (leading
    /// `-`) pass through; every path-like arg must be relative, `..`-free, name no DENY
    /// component, and — if it already exists — canonicalize strictly inside the root
    /// (catches the `g -> .git` symlink trick applied via a command).
    fn args_within_root(&self, argv: &[String]) -> Result<(), String> {
        for a in argv.iter().skip(1) {
            if a.is_empty() || a.starts_with('-') {
                continue;
            }
            let raw = Path::new(a);
            if raw.is_absolute() {
                return Err(format!("absolute path not allowed in command: {a}"));
            }
            for c in raw.components() {
                match c {
                    std::path::Component::ParentDir => {
                        return Err(format!("'..' not allowed in command: {a}"))
                    }
                    std::path::Component::Normal(os) if DENY.iter().any(|d| os == *d) => {
                        return Err(format!("access denied in command: {a}"))
                    }
                    _ => {}
                }
            }
            if let Ok(canon) = std::fs::canonicalize(self.root.join(raw)) {
                if !canon.starts_with(&self.root) || self.deny_hit(&canon) {
                    return Err(format!("path escapes the workspace: {a}"));
                }
            }
        }
        Ok(())
    }

    /// Run an allowlisted, shell-free command in the workspace root with a hard timeout.
    /// The allowlist widens to build/test tools in read-write mode. NOTE: BUILD tools
    /// execute arbitrary project code — `scrub_env` strips credentials and forces offline
    /// defaults, but this is a mitigation, not a security boundary (see `scrub_env`).
    pub async fn run_command(&self, argv: &[String], timeout: Duration) -> Result<CmdResult, String> {
        let prog = argv.first().ok_or("empty command")?;
        if !cmd_allowed(prog, argv, self.mode) {
            return Err(format!("command not allowed: {}", argv.join(" ")));
        }
        self.args_within_root(argv)?;
        use tokio::process::Command;
        let mut cmd = Command::new(prog);
        cmd.args(&argv[1..])
            .current_dir(&self.root)
            .kill_on_drop(true)
            .stdin(std::process::Stdio::null());
        scrub_env(&mut cmd);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW — no black console flash per shell command
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
/// ever; `git push`/`rm`/`curl`/network fetchers are always refused. Path arguments are
/// separately jailed by `Sandbox::args_within_root`, and BUILD tools run under `scrub_env`.
fn cmd_allowed(prog: &str, argv: &[String], mode: SandboxMode) -> bool {
    const READ: &[&str] = &["ls", "cat", "rg", "grep", "find", "wc", "head", "tail", "tree"];
    const BUILD: &[&str] = &[
        "cargo", "rustc", "npm", "pnpm", "yarn", "bun", "node", "deno", "tsc",
        "python", "python3", "pytest", "go", "make", "jest", "mocha", "vitest",
    ];
    let base = Path::new(prog).file_name().and_then(|s| s.to_str()).unwrap_or(prog);
    if base == "git" {
        return git_allowed(argv);
    }
    if READ.contains(&base) {
        return true;
    }
    mode == SandboxMode::ReadWrite && BUILD.contains(&base)
}

/// Read-only git, minus the history-exfiltration vectors. `show`/`blame` are dropped
/// entirely (they dump arbitrary historical blobs/patches); `-p`/`--patch` is refused (a
/// `git log -p` prints committed-then-removed secrets); and any `<ref>:<path>` blob spec is
/// refused (`git show HEAD:.env` reads past the DENY jail). What remains — status, log
/// (metadata), diff (working tree), ls-files, branch, rev-parse, describe — reveals nothing
/// the agent can't already read from the live tree.
fn git_allowed(argv: &[String]) -> bool {
    const GIT_RO: &[&str] =
        &["status", "log", "diff", "ls-files", "branch", "rev-parse", "describe"];
    let sub = match argv.get(1) {
        Some(s) => s.as_str(),
        None => return false,
    };
    if !GIT_RO.contains(&sub) {
        return false;
    }
    for a in argv.iter().skip(2) {
        if a.contains(':') {
            return false; // <ref>:<path> blob extraction
        }
        if matches!(a.as_str(), "-p" | "-u" | "--patch" | "--patch-with-raw" | "--full-diff") {
            return false; // patch dump — leaks removed secrets from history
        }
    }
    true
}

/// Strip credential-bearing env vars and force offline defaults before running a command —
/// especially the BUILD tools, which execute arbitrary project code (build.rs, proc-macros,
/// npm lifecycle scripts, `node -e`/`python -c`). This REDUCES credential exfiltration and
/// network supply-chain reach; it is NOT a substitute for OS-level sandboxing. BUILD mode is
/// not a hardened security boundary and should only be used on trusted workspaces.
fn scrub_env(cmd: &mut tokio::process::Command) {
    for (k, _) in std::env::vars() {
        let up = k.to_ascii_uppercase();
        if up.contains("KEY")
            || up.contains("TOKEN")
            || up.contains("SECRET")
            || up.contains("PASSWORD")
            || up.contains("PASSWD")
            || up.starts_with("AWS_")
            || up.contains("DASHSCOPE")
            || up.contains("DEEPSEEK")
            || up.contains("ANTHROPIC")
            || up.contains("OPENAI")
            || up.starts_with("GH_")
            || up.starts_with("GITHUB_")
            || up == "SSH_AUTH_SOCK"
        {
            cmd.env_remove(&k);
        }
    }
    cmd.env("CARGO_NET_OFFLINE", "true");
    cmd.env("npm_config_offline", "true");
    cmd.env("npm_config_audit", "false");
    cmd.env("npm_config_fund", "false");
    cmd.env("PIP_NO_INPUT", "1");
    cmd.env("GIT_TERMINAL_PROMPT", "0");
}

/// Read at most `max` (+1KB slack so cap_head can detect truncation) bytes of a file into memory,
/// so a giant workspace file can't OOM the host before the byte cap is applied.
fn read_capped(path: &std::path::Path, max: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut buf = Vec::new();
    std::fs::File::open(path)?
        .take((max as u64) + 1024)
        .read_to_end(&mut buf)?;
    Ok(buf)
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

/// Match a filename against a simplified glob token (see `Sandbox::glob`).
fn glob_match(pat: &str, name: &str) -> bool {
    if pat.is_empty() || pat == "*" || pat == "**" {
        return true;
    }
    if let Some(ext) = pat.strip_prefix("*.") {
        return name.ends_with(&format!(".{ext}"));
    }
    let star_pre = pat.starts_with('*');
    let star_suf = pat.ends_with('*');
    let core = pat.trim_matches('*');
    if star_pre && star_suf {
        return !core.is_empty() && name.contains(core);
    }
    if star_pre {
        return name.ends_with(core);
    }
    if star_suf {
        return name.starts_with(core);
    }
    name == pat
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
    fn edit_replaces_unique_refuses_missing_and_ambiguous() {
        let root = tmp("edit");
        std::fs::write(root.join("f.rs"), "let x = 1;\nlet y = 2;\nlet x2 = 1;\n").unwrap();
        let rw = Sandbox::with_mode(&root, SandboxMode::ReadWrite).unwrap();
        // unique match -> replaced
        rw.edit_file("f.rs", "let y = 2;", "let y = 99;").unwrap();
        assert!(rw.read_file("f.rs").unwrap().contains("let y = 99;"));
        // missing -> refused
        assert!(rw.edit_file("f.rs", "let z = 3;", "x").unwrap_err().contains("not found"));
        // ambiguous ("= 1;" appears twice) -> refused, file unchanged
        let before = rw.read_file("f.rs").unwrap();
        assert!(rw.edit_file("f.rs", "= 1;", "= 7;").unwrap_err().contains("matches"));
        assert_eq!(rw.read_file("f.rs").unwrap(), before);
        // read-only mode can't edit
        let ro = Sandbox::new(&root).unwrap();
        assert!(ro.edit_file("f.rs", "let x = 1;", "let x = 0;").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn glob_finds_by_pattern_and_skips_git() {
        let root = tmp("glob");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "x").unwrap();
        std::fs::write(root.join("src/b.rs"), "x").unwrap();
        std::fs::write(root.join("README.md"), "x").unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git/aa.rs"), "x").unwrap();
        let sb = Sandbox::new(&root).unwrap();
        let rs = sb.glob("*.rs").unwrap();
        assert!(rs.contains("src/a.rs") && rs.contains("src/b.rs"), "got: {rs}");
        assert!(!rs.contains("README"), "should not match README");
        assert!(!rs.contains(".git"), ".git must be skipped: {rs}");
        assert!(sb.glob("**/*.md").unwrap().contains("README.md"));
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
    fn git_blocks_history_exfiltration() {
        use SandboxMode::*;
        // safe read-only subcommands
        assert!(cmd_allowed("git", &["git".into(), "status".into()], ReadOnly));
        assert!(cmd_allowed("git", &["git".into(), "log".into()], ReadOnly));
        assert!(cmd_allowed("git", &["git".into(), "diff".into()], ReadOnly));
        // patch dumps leak committed-then-removed secrets
        assert!(!cmd_allowed("git", &["git".into(), "log".into(), "-p".into()], ReadWrite));
        assert!(!cmd_allowed("git", &["git".into(), "log".into(), "--patch".into()], ReadWrite));
        // arbitrary blob reads past the DENY jail
        assert!(!cmd_allowed("git", &["git".into(), "show".into(), "HEAD:.env".into()], ReadWrite));
        assert!(!cmd_allowed("git", &["git".into(), "show".into()], ReadWrite));
        assert!(!cmd_allowed("git", &["git".into(), "blame".into(), "x".into()], ReadWrite));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_to_deny_dir_inside_root_is_blocked() {
        let root = tmp("symdeny");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git").join("config"), "token=secret").unwrap();
        // a symlink whose NAME hides its target: g -> .git (both inside root)
        std::os::unix::fs::symlink(root.join(".git"), root.join("g")).unwrap();
        let sb = Sandbox::with_mode(&root, SandboxMode::ReadWrite).unwrap();
        // read via the symlink must be denied (canonical path hits .git)
        assert!(sb.read_file("g/config").is_err());
        assert!(sb.list_dir("g").is_err());
        // write via the symlink must not land in .git
        assert!(sb.write_file("g/config", "pwn").is_err());
        assert_eq!(
            std::fs::read_to_string(root.join(".git").join("config")).unwrap(),
            "token=secret"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn run_command_args_are_jailed() {
        let root = tmp("argjail");
        std::fs::write(root.join(".env"), "SECRET=1").unwrap();
        std::fs::write(root.join("ok.txt"), "fine").unwrap();
        let sb = Sandbox::new(&root).unwrap();
        let d = Duration::from_secs(5);
        // absolute path out of root
        assert!(sb.run_command(&["cat".into(), "/etc/passwd".into()], d).await.is_err());
        // DENY file inside root, reached by a read command
        assert!(sb.run_command(&["cat".into(), ".env".into()], d).await.is_err());
        // parent escape
        assert!(sb.run_command(&["cat".into(), "../x".into()], d).await.is_err());
        // filesystem-wide search
        assert!(sb.run_command(&["find".into(), "/".into(), "-name".into(), "id_rsa".into()], d).await.is_err());
        // a legitimate in-root read still works
        assert!(sb.run_command(&["cat".into(), "ok.txt".into()], d).await.is_ok());
        let _ = std::fs::remove_dir_all(&root);
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
