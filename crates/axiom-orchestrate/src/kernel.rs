//! A persistent Python kernel — the compute layer that lets an analysis actually RUN
//! (variables/state persist across cells) and produce artifacts (stdout, the last
//! expression's repr, and matplotlib figures captured as PNGs) rather than being asserted.
//! This is the notebook kernel behind the workbench, in the spirit of Claude Science's
//! "persistent Python … variables, dataframes, and loaded models stay in memory."
//!
//! SCOPE / SAFETY: a kernel executes ARBITRARY Python — that is its whole purpose, so it is
//! NOT a security boundary. We scrub credential env vars and run in a chosen working dir,
//! but code the user/agent runs can do anything a Python process can. OS-level sandboxing
//! (seccomp / sandbox-exec / a container) is the follow-up for untrusted use.
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

/// The driver runs inside the Python subprocess: it reads one JSON request per line
/// (`{"id","code"}`), executes the code in a PERSISTENT namespace, captures stdout/stderr +
/// the last expression's repr + any matplotlib figures (as base64 PNG), and writes one
/// `AXKERNEL {json}` line per request back on the real stdout.
const DRIVER: &str = r#"
import sys, json, io, base64, traceback, ast, contextlib
# Default socket timeout so an in-cell download (urllib / pandas.read_csv(url) / requests without a
# timeout) that STALLS fails fast + catchably instead of hanging the whole cell for the full 15-min
# cell budget (uninterruptible). 120s is generous for a real fetch but bounds a dead connection.
try:
    import socket as _socket
    _socket.setdefaulttimeout(120)
except Exception:
    pass
try:
    import matplotlib
    matplotlib.use('Agg')
    # Default CJK-capable fonts so Chinese labels NEVER render as boxes — the model often forgets
    # to set font.sans-serif, and matplotlib's default (DejaVu Sans) has no CJK glyphs. These faces
    # ship on macOS; DejaVu Sans stays as the Latin fallback. Set once at kernel boot, global.
    matplotlib.rcParams['font.sans-serif'] = ['Arial Unicode MS','Heiti TC','STHeiti','PingFang SC','PingFang HK','Songti SC','DejaVu Sans']
    matplotlib.rcParams['axes.unicode_minus'] = False
except Exception:
    pass
NS = {'__name__': '__main__'}
REAL = sys.stdout
def run(code):
    out = io.StringIO(); figs = []; err = None; ok = True
    try:
        block = ast.parse(code, '<cell>', 'exec')
        tail = None
        if block.body and isinstance(block.body[-1], ast.Expr):
            tail = ast.Expression(block.body.pop().value)
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
            exec(compile(block, '<cell>', 'exec'), NS)
            if tail is not None:
                val = eval(compile(tail, '<cell>', 'eval'), NS)
                if val is not None:
                    print(repr(val))
            try:
                import matplotlib.pyplot as plt
                for n in plt.get_fignums():
                    b = io.BytesIO()
                    plt.figure(n).savefig(b, format='png', dpi=110, bbox_inches='tight')
                    figs.append(base64.b64encode(b.getvalue()).decode())
                plt.close('all')
            except Exception:
                pass
    except Exception:
        ok = False; err = traceback.format_exc()
    return {'ok': ok, 'stdout': out.getvalue(), 'error': err, 'figures': figs}
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        req = json.loads(line)
    except Exception:
        continue
    res = run(req.get('code', ''))
    res['id'] = req.get('id')
    REAL.write('AXKERNEL ' + json.dumps(res) + '\n')
    REAL.flush()
"#;

/// One cell's result. `figures` are base64-encoded PNGs.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CellResult {
    pub ok: bool,
    pub stdout: String,
    pub error: Option<String>,
    pub figures: Vec<String>,
}

struct Io {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

/// A live Python kernel process. `exec` is serialized (one cell at a time).
pub struct PyKernel {
    io: Mutex<Io>,
    seq: AtomicU64,
    python: String,
    cwd: std::path::PathBuf, // kept so a timed-out kernel can be respawned in place
}

/// Build + spawn the kernel subprocess, returning its pipes. The child runs in its OWN
/// process group so a runaway cell — and any grandchildren it spawned that inherited the
/// stdout pipe — can be killed as a GROUP on timeout; otherwise a lingering grandchild
/// keeps the pipe's write end open, the reader never sees EOF, and the kernel wedges.
fn spawn_io(python: &str, cwd: &Path) -> anyhow::Result<Io> {
    let mut cmd = Command::new(python);
    cmd.arg("-u") // unbuffered
        .arg("-c")
        .arg(DRIVER)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    scrub_env(&mut cmd);
    // The python.org macOS build ships no usable trust store, so in-cell HTTPS (fetching
    // datasets, PDB files) fails with CERTIFICATE_VERIFY_FAILED. Point it at the system CA
    // bundle unless the caller already set one.
    if std::env::var_os("SSL_CERT_FILE").is_none() {
        for p in ["/etc/ssl/cert.pem", "/etc/ssl/certs/ca-certificates.crt", "/etc/pki/tls/certs/ca-bundle.crt"] {
            if Path::new(p).exists() {
                cmd.env("SSL_CERT_FILE", p);
                break;
            }
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0); // new process group; pgid == the child's pid
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW — the kernel must not flash a console
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("could not start {python}: {e}"))?;
    let stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("no stdin"))?;
    let stdout = BufReader::new(child.stdout.take().ok_or_else(|| anyhow::anyhow!("no stdout"))?);
    Ok(Io { child, stdin, stdout })
}

/// SIGKILL the child's whole process group so grandchildren die too (closing the stdout
/// pipe) — then the reader thread gets EOF instead of blocking forever.
fn kill_group(child: &mut Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        // pgid == pid (process_group(0)); negative target = the whole group.
        unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
    let _ = child.kill();
}

impl PyKernel {
    /// Start a kernel. `python` is the interpreter (e.g. "python3"); `cwd` is where cells run.
    pub fn start(python: &str, cwd: &Path) -> anyhow::Result<Self> {
        let io = spawn_io(python, cwd)?;
        Ok(Self { io: Mutex::new(io), seq: AtomicU64::new(0), python: python.to_string(), cwd: cwd.to_path_buf() })
    }

    pub fn python(&self) -> &str {
        &self.python
    }

    /// Execute one cell, blocking until it finishes or `timeout` elapses. On timeout the
    /// kernel process is killed (a runaway cell can't hang the app) and an error returned.
    pub fn exec(&self, code: &str, timeout: Duration) -> anyhow::Result<CellResult> {
        let id = self.seq.fetch_add(1, Ordering::Relaxed);
        let req = serde_json::json!({ "id": id, "code": code }).to_string();
        let mut guard = self.io.lock().unwrap_or_else(|p| p.into_inner());

        // Kernel already dead (a PRIOR crash/OOM/segfault)? Respawn IN PLACE so it isn't bricked for
        // every future cell, and report the reset so the coding loop auto-restores the last checkpoint.
        if let Ok(Some(_status)) = guard.child.try_wait() {
            let _ = guard.child.wait();
            match spawn_io(&self.python, &self.cwd) {
                Ok(fresh) => *guard = fresh,
                Err(e) => anyhow::bail!("kernel had died and restart failed: {e}"),
            }
            anyhow::bail!("kernel had crashed — restarted; state was reset");
        }
        guard.stdin.write_all(req.as_bytes()).map_err(|e| anyhow::anyhow!("kernel write failed: {e}"))?;
        guard.stdin.write_all(b"\n")?;
        guard.stdin.flush()?;

        // Read the AXKERNEL result line on a worker thread so we can enforce a timeout.
        let mut timed_out = false;
        let mut died = false; // kernel crashed/OOM mid-cell (EOF or read error) — must respawn too
        let outcome: anyhow::Result<CellResult> = {
            // Split the struct into disjoint field borrows so the read thread (stdout) and the
            // timeout kill (child) don't conflict.
            let Io { child, stdin: _, stdout } = &mut *guard;
            let (tx, rx) = mpsc::channel::<std::io::Result<Option<String>>>();
            std::thread::scope(|s| {
                s.spawn(|| {
                    loop {
                        let mut line = String::new();
                        match stdout.read_line(&mut line) {
                            Ok(0) => {
                                let _ = tx.send(Ok(None)); // EOF
                                return;
                            }
                            Ok(_) => {
                                if let Some(rest) = line.strip_prefix("AXKERNEL ") {
                                    let _ = tx.send(Ok(Some(rest.trim_end().to_string())));
                                    return;
                                }
                                // ignore stray output lines
                            }
                            Err(e) => {
                                let _ = tx.send(Err(e));
                                return;
                            }
                        }
                    }
                });
                match rx.recv_timeout(timeout) {
                    Ok(Ok(Some(json))) => parse_result(&json),
                    // EOF / read error mid-cell = the kernel process DIED (segfault, OOM-kill, a C
                    // extension crash). Previously this bailed WITHOUT respawning, so the dead handle
                    // stayed installed and every later cell failed forever ("kernel exited") — a heavy
                    // run was silently bricked. Flag it so we respawn just like a timeout.
                    Ok(Ok(None)) => { died = true; anyhow::bail!("kernel closed unexpectedly (crash/OOM)") }
                    Ok(Err(e)) => { died = true; anyhow::bail!("kernel read failed: {e}") }
                    Err(_) => {
                        // Kill the whole process GROUP (not just the child) so grandchildren
                        // die and the stdout pipe closes — otherwise the scoped reader below
                        // would never see EOF and the scope join would deadlock the kernel.
                        kill_group(child);
                        timed_out = true;
                        anyhow::bail!("cell timed out after {}s", timeout.as_secs())
                    }
                }
            })
        };

        if timed_out || died {
            // Respawn a fresh interpreter IN PLACE so the kernel stays usable (the previous version
            // killed/lost the child but left the dead handle installed, bricking every later cell).
            // Reap the old child first to avoid a zombie. Report "state was reset" either way so the
            // coding loop's auto-restore reloads the last checkpoint.
            let _ = guard.child.wait();
            match spawn_io(&self.python, &self.cwd) {
                Ok(fresh) => *guard = fresh,
                Err(e) => anyhow::bail!(
                    "kernel {}; restart failed: {e}",
                    if timed_out { format!("timed out after {}s", timeout.as_secs()) } else { "crashed".into() }
                ),
            }
            if timed_out {
                anyhow::bail!("cell timed out after {}s (kernel restarted — state was reset)", timeout.as_secs());
            }
            anyhow::bail!("kernel crashed (likely OOM/segfault) — restarted; state was reset");
        }
        outcome
    }
}

impl Drop for PyKernel {
    fn drop(&mut self) {
        let mut io = self.io.lock().unwrap_or_else(|p| p.into_inner());
        kill_group(&mut io.child);
        let _ = io.child.wait(); // reap so we don't leave a zombie
    }
}

fn parse_result(json: &str) -> anyhow::Result<CellResult> {
    #[derive(serde::Deserialize)]
    struct Raw {
        ok: bool,
        stdout: String,
        error: Option<String>,
        figures: Vec<String>,
    }
    let r: Raw = serde_json::from_str(json).map_err(|e| anyhow::anyhow!("bad kernel result: {e}"))?;
    Ok(CellResult { ok: r.ok, stdout: r.stdout, error: r.error, figures: r.figures })
}

/// Strip credential env vars before running Python (a cell shouldn't inherit provider keys).
fn scrub_env(cmd: &mut Command) {
    // vars_os (not vars) so an unusual non-UTF8 env var name can't panic credential scrubbing.
    for (k, _) in std::env::vars_os() {
        let up = k.to_string_lossy().to_ascii_uppercase();
        if up.contains("KEY") || up.contains("TOKEN") || up.contains("SECRET") || up.contains("PASSWORD")
            || up.starts_with("AWS_") || up.contains("DEEPSEEK") || up.contains("ANTHROPIC")
            || up.contains("OPENAI") || up.contains("DASHSCOPE") || up == "SSH_AUTH_SOCK"
        {
            cmd.env_remove(&k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn have_python() -> Option<String> {
        for p in ["python3", "python"] {
            if Command::new(p).arg("--version").output().map(|o| o.status.success()).unwrap_or(false) {
                return Some(p.to_string());
            }
        }
        None
    }

    #[test]
    fn persistent_state_and_stdout() {
        let Some(py) = have_python() else { return }; // skip where no python
        let k = PyKernel::start(&py, std::env::temp_dir().as_path()).unwrap();
        let r1 = k.exec("x = 40\nprint('hi')", Duration::from_secs(20)).unwrap();
        assert!(r1.ok, "err: {:?}", r1.error);
        assert!(r1.stdout.contains("hi"));
        // state persists across cells
        let r2 = k.exec("x + 2", Duration::from_secs(20)).unwrap();
        assert!(r2.ok);
        assert!(r2.stdout.contains("42"), "stdout was {:?}", r2.stdout);
    }

    #[test]
    fn crash_respawns_kernel_and_stays_usable() {
        let Some(py) = have_python() else { return };
        let k = PyKernel::start(&py, std::env::temp_dir().as_path()).unwrap();
        k.exec("x = 5", Duration::from_secs(20)).unwrap();
        // Hard-crash the kernel process (os._exit bypasses cleanup → EOF on our pipe, like an OOM
        // SIGKILL / C-extension segfault). This must NOT brick the kernel.
        let r = k.exec("import os; os._exit(1)", Duration::from_secs(20));
        assert!(r.is_err(), "a kernel crash should surface as an error, got {r:?}");
        let msg = format!("{:?}", r.unwrap_err());
        assert!(msg.contains("state was reset") || msg.contains("crashed"), "crash error should signal a reset: {msg}");
        // The kernel must have respawned and be usable again (state is expected to be gone).
        let r2 = k.exec("print(2 + 2)", Duration::from_secs(20)).unwrap();
        assert!(r2.ok && r2.stdout.contains("4"), "kernel must work after a crash: {r2:?}");
    }

    #[test]
    fn error_is_captured_not_fatal() {
        let Some(py) = have_python() else { return };
        let k = PyKernel::start(&py, std::env::temp_dir().as_path()).unwrap();
        let r = k.exec("1/0", Duration::from_secs(20)).unwrap();
        assert!(!r.ok);
        assert!(r.error.as_deref().unwrap_or("").contains("ZeroDivisionError"));
        // kernel survives the error
        let r2 = k.exec("print('still alive')", Duration::from_secs(20)).unwrap();
        assert!(r2.ok && r2.stdout.contains("still alive"));
    }

    #[test]
    fn timeout_restarts_kernel_and_stays_usable() {
        let Some(py) = have_python() else { return };
        let k = PyKernel::start(&py, std::env::temp_dir().as_path()).unwrap();
        // A runaway cell — must time out, not hang the test.
        let r = k.exec("while True:\n    pass", Duration::from_millis(600));
        assert!(r.is_err(), "runaway cell should time out");
        assert!(r.unwrap_err().to_string().contains("timed out"));
        // The kernel must be RESTARTED (usable), not bricked — the old bug left a dead child
        // so every later cell returned "kernel exited".
        let r2 = k.exec("print(6 * 7)", Duration::from_secs(20)).unwrap();
        assert!(r2.ok, "kernel must work after a timeout, err: {:?}", r2.error);
        assert!(r2.stdout.contains("42"), "stdout after restart: {:?}", r2.stdout);
    }

    #[test]
    fn timeout_with_pipe_inheriting_grandchild_does_not_deadlock() {
        let Some(py) = have_python() else { return };
        let k = PyKernel::start(&py, std::env::temp_dir().as_path()).unwrap();
        // A background grandchild inherits stdout; on the old code its open pipe write-end
        // kept the reader blocked so the scope join deadlocked. The group-kill must free it.
        let code = "import subprocess, sys\nsubprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])\nwhile True:\n    pass";
        let r = k.exec(code, Duration::from_millis(800));
        assert!(r.is_err(), "should time out, not deadlock");
        // And still usable after.
        let r2 = k.exec("print('ok')", Duration::from_secs(20)).unwrap();
        assert!(r2.ok && r2.stdout.contains("ok"));
    }
}
