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
_AX_SAVED = []   # (id(fig), base64 png, has_data) for every figure the current cell explicitly savefig()'d
def _ax_has_data(fig):
    # A figure is "blank" if NO axes holds real data — a >=3-point curve, a scatter/collection, an
    # image, bars, or an errorbar/bar container. 2-point reference lines (axhline/axvline) and bare
    # grids/labels/titles do NOT count, so an all-scaffold-no-curve panel (empty data / failed fit,
    # like the blank dose-response grids users saw) is caught and dropped.
    #
    # The >=3-point rule was a proxy for "this line is data, not an axhline". It is not needed: a
    # reference line drawn by axhline/axvline uses a BLENDED transform, while a real data line is on
    # ax.transData. Testing the transform directly lets an honest 2-point line (a paired before/after
    # slope chart) count, which the old rule silently discarded.
    try:
        for ax in fig.get_axes():
            for ln in ax.get_lines():
                xd = ln.get_xdata()
                if xd is not None and len(xd) >= 2 and ln.get_transform() is ax.transData:
                    return True
            if ax.collections or ax.images or ax.patches or getattr(ax, 'containers', None):
                return True
            # A table figure. `ax.table(...)` lands in ax.tables and touches nothing else, so a
            # descriptor/parameter table was being dropped as "blank".
            if getattr(ax, 'tables', None):
                return True
            # A schematic drawn with annotate arrows — the pathway/mechanism diagram. The arrow of an
            # annotation is NOT added to ax.patches; it hangs off the Annotation as `arrow_patch`.
            # This is why a requested 机制图 could vanish from a report with no error anywhere.
            if any(getattr(t, 'arrow_patch', None) is not None for t in getattr(ax, 'texts', ())):
                return True
            # Axes turned off + text = a deliberate label-only diagram, not an empty panel.
            if not getattr(ax, 'axison', True) and getattr(ax, 'texts', None):
                return True
        return False
    except Exception:
        return True   # fail-open: on any inspection error, KEEP the figure (never lose a real one)
_AX_OVER = []    # (w, h) of oversize figures RECOVERED by refitting
_AX_DROP = []    # (w, h) of oversize figures that could not be recovered and were dropped
_AX_SEEN = set() # ids handled by the savefig hook — including drops, so the open-figure sweep
                 # below never renders the same figure twice
_AX_KEEP = []    # strong refs to this cell's captured figures. CPython reuses the memory of a freed
                 # object, so `fig.savefig(p); plt.close(fig)` followed by a new figure could hand the
                 # new one the SAME id() — and it would be skipped as "already captured". Holding a
                 # reference for the cell's lifetime makes that collision impossible. (Measured: 3 of
                 # 160 figures lost over 80 save-then-leave-open rounds; 0 after.)
_AX_MAXPX = 12000  # per-side cap. WebKit refuses to decode much beyond ~32767px; no honest figure
                   # is anywhere near 12000, so this only ever catches a runaway canvas.
def _ax_dims(_png):
    # (width, height) straight out of the PNG IHDR — a 24-byte header read, NOT a decode. This must
    # stay decode-free: the images it exists to catch are hundreds of megapixels, and decoding one
    # costs gigabytes (matplotlib's imread returns float32 RGBA — a 1059x298982 PNG is ~5 GB).
    try:
        if _png[:8] != b'\x89PNG\r\n\x1a\n':
            return None
        import struct as _st
        return _st.unpack('>II', _png[16:24])
    except Exception:
        return None
def _ax_fit(_fig, _png, _orig):
    # Reject a canvas no renderer can draw, and try to recover the figure inside it.
    #
    # Observed in the wild: a scatter with `ax.set_yscale('log')` annotated via DATA coordinates —
    # `xytext=(x + dx, y + dy)` — where dy pushed y negative. A negative coordinate is undefined on a
    # log axis, so the text landed effectively infinitely far down, and `bbox_inches='tight'` did its
    # job faithfully: it grew the canvas until the stray label fitted. Result 1059x298982 px, 2.6 MB
    # of base64 in the report, and a broken-image icon for the user — while the agent was told the
    # figure saved fine and moved on.
    #
    # The plot itself is intact; only the tight-bbox expansion is broken. So re-render at the
    # figure's own figsize (no tight), which clips the stray artist and keeps the chart.
    _d = _ax_dims(_png)
    if not _d:
        return _png
    _w, _h = _d
    if _w <= _AX_MAXPX and _h <= _AX_MAXPX:
        return _png
    try:
        _b2 = io.BytesIO(); _orig(_fig, _b2, format='png', dpi=110)
        _p2 = _b2.getvalue()
        _d2 = _ax_dims(_p2)
        if _d2 and _d2[0] <= _AX_MAXPX and _d2[1] <= _AX_MAXPX:
            _AX_OVER.append((_w, _h))
            return _p2
    except Exception:
        pass
    # Still oversize without tight → the figure's OWN figsize is the problem (a 900-row heatmap at
    # figsize=(10, 0.2*n) is >180 inches tall). Nothing to recover; record it separately so the agent
    # is told the figure is GONE and why, instead of the refit message, which would have it hunting a
    # log-axis annotation that does not exist while believing the figure shipped.
    _AX_DROP.append((_w, _h))
    return None   # drop it rather than ship an image that cannot be displayed
def _ax_trim(_png):
    # Crop a LARGE blank band from the top/bottom of a captured figure. bbox_inches='tight' already
    # removes outer margins, but an over-tall axes (y-limits with empty rows) or a stray low artist
    # leaves a big blank band INSIDE the figure that tight can't touch — the "超长图 + 下方全是空白"
    # case. We measure per-row ink: a row with only the axes spines / dashed reference line (a few px)
    # counts as blank; a real data/label/title row does not. Only the CONTIGUOUS top & bottom blank
    # runs are trimmed (never interior), a generous margin is kept, and we bail unless the band is big —
    # so no data, label, or marker near an edge is ever clipped. Fail-open: return input on any error.
    try:
        import numpy as _np, matplotlib.image as _mi
        _a = _mi.imread(io.BytesIO(_png), format='png')
        if _a.ndim != 3 or _a.shape[2] < 3:
            return _png
        H, W = _a.shape[0], _a.shape[1]
        _ink = (_a[:, :, :3] < 0.90).mean(axis=(1, 2))   # per-row fraction of non-white pixels, shape (H,)
        _blank = _ink < 0.012                            # spine/reference-line-only (or empty) rows
        _b = H
        while _b > 0 and _blank[_b - 1]:
            _b -= 1
        _t = 0
        while _t < H and _blank[_t]:
            _t += 1
        _pad = max(10, int(round(H * 0.01)))
        _t = max(0, _t - _pad)
        _b = min(H, _b + _pad)
        removed = _t + (H - _b)
        if removed < max(24, 0.05 * H):                  # only bother with a MEANINGFUL blank band
            return _png
        if _b - _t < 60:                                 # never collapse the image
            return _png
        _o = io.BytesIO(); _mi.imsave(_o, _a[_t:_b, :, :], format='png', dpi=110)
        return _o.getvalue()
    except Exception:
        return _png
try:
    import matplotlib
    matplotlib.use('Agg')
    # Default CJK-capable fonts so Chinese labels NEVER render as boxes — the model often forgets to
    # set font.sans-serif, and matplotlib's default (DejaVu Sans) has no CJK glyphs. Cover EVERY OS:
    # macOS (PingFang/Heiti), Windows (Microsoft YaHei / SimHei / SimSun), Linux (Noto / WenQuanYi).
    # Without the Windows faces, figure labels boxed on the Windows build. DejaVu Sans = Latin fallback.
    matplotlib.rcParams['font.sans-serif'] = ['Arial Unicode MS','Heiti TC','STHeiti','PingFang SC','PingFang HK','Songti SC','Microsoft YaHei','SimHei','SimSun','Noto Sans CJK SC','WenQuanYi Zen Hei','DejaVu Sans']
    matplotlib.rcParams['axes.unicode_minus'] = False
    # Capture figures the code explicitly saves — EVEN IF it then plt.close()s them. The structure /
    # composition pipelines do `savefig(path); plt.close()`, which get_fignums() (open figures only)
    # can't see, so the figure was drawn to disk but "not loaded" into the report. Hooking savefig
    # records the bytes at save time, so a saved-then-closed figure still lands inline.
    import matplotlib.figure as _mfig
    if not getattr(_mfig.Figure, '_ax_hooked', False):
        _mfig.Figure._ax_orig = _mfig.Figure.savefig
        def _ax_savefig(self, *a, **k):
            _r = _mfig.Figure._ax_orig(self, *a, **k)
            try:
                _bb = io.BytesIO(); _mfig.Figure._ax_orig(self, _bb, format='png', dpi=110, bbox_inches='tight')
                # Size guard BEFORE trim: trim decodes the image, and the runaway canvases this
                # catches are big enough to exhaust memory during that decode (whereupon trim would
                # fail open and pass the monster straight through).
                _pf = _ax_fit(self, _bb.getvalue(), _mfig.Figure._ax_orig)
                # Register the id even when the figure is DROPPED, so the still-open-figure sweep
                # below does not pick the same figure up again and render it a second time — which
                # would double-count the oversize warning and re-run the very rendering that is
                # expensive enough to be an OOM risk.
                _AX_SEEN.add(id(self)); _AX_KEEP.append(self)
                if _pf is not None:
                    _AX_SAVED.append((id(self), base64.b64encode(_ax_trim(_pf)).decode(), _ax_has_data(self)))
            except Exception:
                pass
            return _r
        _mfig.Figure.savefig = _ax_savefig
        _mfig.Figure._ax_hooked = True
except Exception:
    pass
NS = {'__name__': '__main__'}
REAL = sys.stdout
def run(code):
    out = io.StringIO(); figs = []; err = None; ok = True
    del _AX_SAVED[:]   # per-cell reset (in place so the savefig hook keeps its reference)
    del _AX_OVER[:]; del _AX_DROP[:]; del _AX_KEEP[:]; _AX_SEEN.clear()
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
                import matplotlib.figure as _mfig
                seen = set(); _blank = 0   # dedup by PNG bytes; count dropped blank (no-data) figures
                # 1) figures the code explicitly saved (survive plt.close())
                saved_ids = set()
                for _fid, _d, _hd in _AX_SAVED:
                    saved_ids.add(_fid)
                    if _d in seen: continue
                    if not _hd: _blank += 1; continue   # scaffold-only / empty-data figure → drop it
                    figs.append(_d); seen.add(_d)
                # 2) any still-open figure that was never saved (e.g. just left open / plt.show())
                _orig = getattr(_mfig.Figure, '_ax_orig', None)
                _bad = 0
                # Per-figure isolation + an unconditional close. Rendering happens HERE, not when the
                # cell drew the figure, so a malformed label (e.g. broken mathtext in a title) raises
                # only now. Without this the exception escaped the whole block, `plt.close('all')`
                # never ran, and the unrenderable figure stayed in matplotlib's registry — so it threw
                # again on the NEXT cell, and the next: every later figure silently vanished from the
                # report while each step still reported success.
                try:
                    for n in plt.get_fignums():
                        f = plt.figure(n)
                        if id(f) in saved_ids or id(f) in _AX_SEEN: continue
                        try:
                            if not _ax_has_data(f): _blank += 1; continue   # drop a blank open figure
                            b = io.BytesIO()
                            _sv = _orig or _mfig.Figure.savefig
                            _sv(f, b, format='png', dpi=110, bbox_inches='tight')
                            _pf = _ax_fit(f, b.getvalue(), _sv)
                            if _pf is None: continue
                            _d = base64.b64encode(_ax_trim(_pf)).decode()
                            if _d not in seen: figs.append(_d); seen.add(_d)
                        except Exception:
                            _bad += 1
                finally:
                    plt.close('all')
                if _blank:
                    # surfaced in the cell's stdout → the agent SELF-CHECKS and redraws (fault tolerance)
                    print("\n[axiom] ⚠ 丢弃了 %d 张空图（坐标轴内没有任何数据曲线/散点/柱）。多半是取到的数据为空或拟合失败——请先确认数据非空再重画；某分组确实无数据就明确标注'无数据'，不要输出空白子图。" % _blank)
                if _bad:
                    print("\n[axiom] ⚠ %d 张图在渲染时报错、已跳过（图形本身画得出，但存成图片时失败）。最常见成因是标签里的 mathtext 写坏了（如 $\\frac{1}{}$、未配对的 $），matplotlib 直到渲染这一刻才解析它。请检查标题/轴标签/图例里的 $...$ 片段后重画。" % _bad)
                if _AX_DROP:
                    _whd = "、".join("%dx%d" % _t for _t in _AX_DROP)
                    print("\n[axiom] ⚠ %d 张图因自身 figsize 过大被【整张丢弃】，报告里没有这张图（重渲染后仍为 %s 像素，超过 12000px 上限）。请把 figsize 高/宽控制在 100 英寸以内，或拆成多张图重画。" % (len(_AX_DROP), _whd))
                if _AX_OVER:
                    # Same contract as the blank-figure notice: tell the agent, in the cell's own
                    # stdout, what went wrong and how to redraw. Without this the failure is silent —
                    # savefig returns normally, so the agent believes the figure shipped.
                    _wh = "、".join("%dx%d" % _t for _t in _AX_OVER)
                    print("\n[axiom] ⚠ %d 张图的画布被撑到 %s 像素，超出可渲染上限（浏览器画不出来，会显示成破图）。已按图自身 figsize 重新截取，画布外的标注被裁掉。\n  成因几乎总是把标注放到了坐标轴外：在对数轴上用【数据坐标】做加减偏移（如 xytext=(x+dx, y-10)，y-10 变成负数——负值在 log 轴上无定义），bbox_inches='tight' 便会把画布扩张到能装下它。\n  改法：偏移用【点】而不是数据坐标 —— ax.annotate(text, (x, y), textcoords='offset points', xytext=(6, 6))。请据此重画并确认标注都在图内。" % (len(_AX_OVER), _wh))
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
/// Axiom3's pre-configured scientific-database toolkit, embedded at compile time and dropped onto
/// the kernel's PYTHONPATH so every analysis can `import scidb` (UniProt / PDB / PubChem / ChEMBL /
/// NCBI Entrez / Ensembl / KEGG / Reactome / STRING + literature) — real records, cited, never faked.
const SCIDB_PY: &str = include_str!("scidb.py");

/// Materialise `scidb.py` at a stable, env-independent location and return the dir to put on
/// PYTHONPATH. Rewritten only when missing or changed, so kernel starts don't thrash the file.
fn ensure_pylib() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let dir = std::path::Path::new(&home).join(".axiom3").join("pylib");
    std::fs::create_dir_all(&dir).ok()?;
    let f = dir.join("scidb.py");
    if std::fs::read_to_string(&f).ok().as_deref() != Some(SCIDB_PY) {
        std::fs::write(&f, SCIDB_PY).ok()?;
    }
    Some(dir)
}

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
    // Put the bundled `scidb` toolkit on PYTHONPATH (after scrub_env, which only strips creds).
    if let Some(pylib) = ensure_pylib() {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let joined = match std::env::var("PYTHONPATH") {
            Ok(p) if !p.is_empty() => format!("{}{}{}", pylib.display(), sep, p),
            _ => pylib.display().to_string(),
        };
        cmd.env("PYTHONPATH", joined);
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

    /// An interpreter that can `import matplotlib` — the figure tests are meaningless without one,
    /// and the bare system python3 usually has no matplotlib. Checks the app's managed kernel venv
    /// too, which is where the real runs get theirs.
    fn have_matplotlib_python() -> Option<String> {
        let mut cands: Vec<String> = Vec::new();
        if let Ok(p) = std::env::var("AXIOM_TEST_PYTHON") {
            cands.push(p);
        }
        if let Some(h) = std::env::var_os("HOME") {
            let h = std::path::Path::new(&h);
            cands.push(
                h.join("Library/Application Support/com.axiom3.desktop/notebook/.venv/bin/python")
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        cands.extend(["python3".to_string(), "python".to_string()]);
        cands.into_iter().find(|p| {
            Command::new(p)
                .args(["-c", "import matplotlib"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        })
    }

    /// (width, height) from a base64 PNG, read out of the IHDR. Only the first 24 bytes are needed,
    /// so this decodes just the leading base64 quantums instead of pulling in a decoder dependency.
    fn png_dims_b64(b64: &str) -> Option<(u32, u32)> {
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut bytes = Vec::new();
        let mut acc: u32 = 0;
        let mut bits = 0;
        for c in b64.bytes() {
            let Some(v) = A.iter().position(|&a| a == c) else { continue };
            acc = (acc << 6) | v as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                bytes.push((acc >> bits) as u8);
                if bytes.len() >= 24 {
                    break;
                }
            }
        }
        if bytes.len() < 24 {
            return None;
        }
        let g = |o: usize| u32::from_be_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
        Some((g(16), g(20)))
    }

    /// REGRESSION (real run, 2026-07-21): a potency/selectivity scatter annotated in DATA
    /// coordinates on a log y-axis — `xytext=(x+dx, y-10)` where y=2.1 — put its label at a negative
    /// y, which is undefined on a log scale. `bbox_inches='tight'` then grew the canvas until that
    /// label fitted: 1059x298982 px, 2.6 MB of base64 in the report, a broken-image icon for the
    /// user, and no error anywhere — savefig returned normally, so the agent moved on believing the
    /// figure had shipped.
    ///
    /// The pre-existing `_ax_trim` could not save this: the stray labels sit on the very LAST rows,
    /// so the contiguous-from-the-bottom blank run is zero-length and the huge blank band is
    /// interior — which trim deliberately never touches.
    #[test]
    fn runaway_canvas_is_refitted_and_reported_not_shipped() {
        let Some(py) = have_matplotlib_python() else { return };
        let k = PyKernel::start(&py, std::env::temp_dir().as_path()).unwrap();
        let code = r#"
import matplotlib; matplotlib.use('Agg')
import matplotlib.pyplot as plt
fig, ax = plt.subplots(figsize=(9, 6.5))
ax.scatter([0.02, 0.1, 5.0, 7.0], [100.0, 50.0, 4.0, 2.1])
ax.set_xscale('log'); ax.set_yscale('log')
# The bug, verbatim: an offset in DATA coordinates that drives y negative on a log axis.
ax.annotate('Sorafenib', (7.0, 2.1), xytext=(7.0 + 1, 2.1 - 10))
ax.annotate('Sulfasalazine', (5.0, 4.0), xytext=(5.0 + 15, 4.0 - 8))
plt.savefig('runaway.png', dpi=140, bbox_inches='tight')
"#;
        let r = k.exec(code, Duration::from_secs(180)).unwrap();
        assert!(r.ok, "cell should still succeed: {:?}", r.error);
        assert_eq!(r.figures.len(), 1, "the chart itself is fine and must survive, refitted");

        let (w, h) = png_dims_b64(&r.figures[0]).expect("captured figure must be a readable PNG");
        assert!(
            w <= 12000 && h <= 12000,
            "a figure no renderer can draw must never be shipped — got {w}x{h}"
        );
        // Silent recovery would be its own bug: the agent has to learn the annotation is wrong,
        // otherwise it keeps drawing the same broken figure.
        assert!(
            r.stdout.contains("超出可渲染上限") && r.stdout.contains("offset points"),
            "the agent must be told what broke and how to redraw: {:?}",
            r.stdout
        );
    }

    /// A figure that raises while being RENDERED used to poison the capture path permanently.
    /// Rendering happens at capture time, not draw time, so a broken mathtext label sails through the
    /// cell that made it and blows up here; the exception escaped the whole block, `plt.close('all')`
    /// never ran, and the bad figure stayed in matplotlib's registry — throwing again on every later
    /// cell. Symptom: from some step onward the report simply has no figures, and every step still
    /// says it succeeded.
    #[test]
    fn a_figure_that_fails_to_render_cannot_poison_later_cells() {
        let Some(py) = have_matplotlib_python() else { return };
        let k = PyKernel::start(&py, std::env::temp_dir().as_path()).unwrap();
        let bad = r#"
import matplotlib; matplotlib.use('Agg')
import matplotlib.pyplot as plt
fig, ax = plt.subplots()
ax.plot([1, 2, 3], [1, 4, 9])
ax.set_title(r'$\frac{1}{}$')   # malformed mathtext — only parsed at render time
"#;
        let r1 = k.exec(bad, Duration::from_secs(180)).unwrap();
        assert!(r1.ok, "the cell itself must not fail: {:?}", r1.error);
        assert!(
            r1.stdout.contains("渲染时报错"),
            "a figure that could not be rendered must be reported, not silently dropped: {:?}",
            r1.stdout
        );

        // The real regression: the NEXT cell's perfectly good figure must still be captured.
        let good = r#"
import matplotlib.pyplot as plt
fig, ax = plt.subplots()
ax.plot([1, 2, 3], [2, 4, 6])
"#;
        let r2 = k.exec(good, Duration::from_secs(180)).unwrap();
        assert_eq!(r2.figures.len(), 1, "a later cell's figure must survive an earlier bad one");
    }

    /// `_ax_has_data` decides what counts as a real figure. Its old proxy (a line with >=3 points)
    /// discarded three legitimate kinds — and did so silently, telling the agent the figure was
    /// "blank" so it never understood what to fix. The mechanism/pathway schematic is the one that
    /// hurt: a real run was asked for a 机制图 and the report shipped without one.
    #[test]
    fn schematic_table_and_two_point_figures_are_not_mistaken_for_blank() {
        let Some(py) = have_matplotlib_python() else { return };
        let k = PyKernel::start(&py, std::env::temp_dir().as_path()).unwrap();
        let cases: &[(&str, &str)] = &[
            ("annotate 箭头机制图", r#"
import matplotlib; matplotlib.use('Agg')
import matplotlib.pyplot as plt
fig, ax = plt.subplots()
ax.annotate('', xy=(0.8, 0.5), xytext=(0.2, 0.5), arrowprops=dict(arrowstyle='->'))
ax.text(0.2, 0.55, 'SLC7A11'); ax.text(0.8, 0.55, 'GPX4')
ax.axis('off')
"#),
            ("ax.table 表格图", r#"
import matplotlib.pyplot as plt
fig, ax = plt.subplots()
ax.axis('off')
ax.table(cellText=[['MW', '421.3'], ['clogP', '3.08']], loc='center')
"#),
            ("两点配对斜率图", r#"
import matplotlib.pyplot as plt
fig, ax = plt.subplots()
for a, b in [(1.0, 2.0), (1.5, 2.4), (0.8, 1.9)]:
    ax.plot([0, 1], [a, b], 'o-')
"#),
        ];
        for (name, code) in cases {
            let r = k.exec(code, Duration::from_secs(180)).unwrap();
            assert!(r.ok, "{name} 应能执行: {:?}", r.error);
            assert_eq!(r.figures.len(), 1, "{name} 被误判为空图丢弃了。stdout: {:?}", r.stdout);
        }
    }

    /// An axhline is still not data: a panel with nothing but reference lines and labels must stay
    /// classified as blank, or the relaxed 2-point rule would let empty scaffolding through.
    #[test]
    fn reference_lines_alone_still_count_as_blank() {
        let Some(py) = have_matplotlib_python() else { return };
        let k = PyKernel::start(&py, std::env::temp_dir().as_path()).unwrap();
        let code = r#"
import matplotlib; matplotlib.use('Agg')
import matplotlib.pyplot as plt
fig, ax = plt.subplots()
ax.axhline(y=1.0); ax.axvline(x=0.5)
ax.set_xlabel('x'); ax.set_ylabel('y'); ax.set_title('no data here')
"#;
        let r = k.exec(code, Duration::from_secs(180)).unwrap();
        assert_eq!(r.figures.len(), 0, "只有参考线的空面板不该算图: {:?}", r.stdout);
        assert!(r.stdout.contains("空图"), "空图必须回告智能体: {:?}", r.stdout);
    }

    /// A figure too big because of its OWN figsize cannot be refitted. It must be reported as DROPPED
    /// (the report has no such figure) rather than with the refit message, which would send the agent
    /// hunting a log-axis annotation that does not exist while believing the figure had shipped.
    #[test]
    fn an_unrecoverably_large_figure_is_reported_as_dropped_not_refitted() {
        let Some(py) = have_matplotlib_python() else { return };
        let k = PyKernel::start(&py, std::env::temp_dir().as_path()).unwrap();
        let code = r#"
import matplotlib; matplotlib.use('Agg')
import matplotlib.pyplot as plt
fig, ax = plt.subplots(figsize=(10, 200))   # 200in * 110dpi = 22000px, over the cap on its own
ax.plot([1, 2, 3], [1, 4, 9])
fig.savefig('huge.png', dpi=110, bbox_inches='tight')
"#;
        let r = k.exec(code, Duration::from_secs(300)).unwrap();
        assert_eq!(r.figures.len(), 0, "救不回来的图必须丢弃,不能发一张画不出的图");
        assert!(r.stdout.contains("整张丢弃"), "必须说清这张图没进报告: {:?}", r.stdout);
        assert!(!r.stdout.contains("重新截取"), "不能报成「已重新截取」: {:?}", r.stdout);
        // Counted once, not twice — the drop path must still register the figure id.
        assert_eq!(r.stdout.matches("整张丢弃").count(), 1, "同一张图被处理了两遍: {:?}", r.stdout);
    }

    /// The guard must stay out of the way of ordinary figures — no refit, no warning.
    #[test]
    fn normal_figure_is_untouched_by_the_size_guard() {
        let Some(py) = have_matplotlib_python() else { return };
        let k = PyKernel::start(&py, std::env::temp_dir().as_path()).unwrap();
        let code = r#"
import matplotlib; matplotlib.use('Agg')
import matplotlib.pyplot as plt
fig, ax = plt.subplots(figsize=(9, 6.5))
ax.scatter([0.02, 0.1, 5.0, 7.0], [100.0, 50.0, 4.0, 2.1])
ax.set_xscale('log'); ax.set_yscale('log')
ax.annotate('ok', (7.0, 2.1), textcoords='offset points', xytext=(6, 6))
plt.savefig('fine.png', dpi=140, bbox_inches='tight')
"#;
        let r = k.exec(code, Duration::from_secs(180)).unwrap();
        assert!(r.ok, "err: {:?}", r.error);
        assert_eq!(r.figures.len(), 1);
        let (w, h) = png_dims_b64(&r.figures[0]).unwrap();
        assert!(w < 3000 && h < 3000, "a normal figure should stay normal: {w}x{h}");
        assert!(!r.stdout.contains("超出可渲染上限"), "must not warn on a fine figure: {:?}", r.stdout);
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
