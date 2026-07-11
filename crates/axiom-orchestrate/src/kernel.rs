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
try:
    import matplotlib
    matplotlib.use('Agg')
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
}

impl PyKernel {
    /// Start a kernel. `python` is the interpreter (e.g. "python3"); `cwd` is where cells run.
    pub fn start(python: &str, cwd: &Path) -> anyhow::Result<Self> {
        let mut cmd = Command::new(python);
        cmd.arg("-u") // unbuffered
            .arg("-c")
            .arg(DRIVER)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        scrub_env(&mut cmd);
        // The python.org macOS build ships no usable trust store, so in-cell HTTPS
        // (fetching datasets, PDB files) fails with CERTIFICATE_VERIFY_FAILED. Point it
        // at the system CA bundle unless the caller already set one.
        if std::env::var_os("SSL_CERT_FILE").is_none() {
            for p in ["/etc/ssl/cert.pem", "/etc/ssl/certs/ca-certificates.crt", "/etc/pki/tls/certs/ca-bundle.crt"] {
                if Path::new(p).exists() {
                    cmd.env("SSL_CERT_FILE", p);
                    break;
                }
            }
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("could not start {python}: {e}"))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("no stdin"))?;
        let stdout = BufReader::new(child.stdout.take().ok_or_else(|| anyhow::anyhow!("no stdout"))?);
        Ok(Self { io: Mutex::new(Io { child, stdin, stdout }), seq: AtomicU64::new(0), python: python.to_string() })
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
        // Split the struct into disjoint field borrows so the read thread (stdout) and the
        // timeout kill (child) don't conflict.
        let Io { child, stdin, stdout } = &mut *guard;

        // is the child still alive?
        if let Ok(Some(status)) = child.try_wait() {
            anyhow::bail!("kernel exited ({status})");
        }
        stdin.write_all(req.as_bytes()).map_err(|e| anyhow::anyhow!("kernel write failed: {e}"))?;
        stdin.write_all(b"\n")?;
        stdin.flush()?;

        // Read the AXKERNEL result line on a worker thread so we can enforce a timeout.
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
                Ok(Ok(None)) => anyhow::bail!("kernel closed unexpectedly"),
                Ok(Err(e)) => anyhow::bail!("kernel read failed: {e}"),
                Err(_) => {
                    let _ = child.kill();
                    anyhow::bail!("cell timed out after {}s (kernel restarted)", timeout.as_secs());
                }
            }
        })
    }
}

impl Drop for PyKernel {
    fn drop(&mut self) {
        if let Ok(mut io) = self.io.lock() {
            let _ = io.child.kill();
        }
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
    for (k, _) in std::env::vars() {
        let up = k.to_ascii_uppercase();
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
}
