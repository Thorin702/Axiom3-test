//! Verifies #3 conda_install synchronous mechanism: micromamba bootstrap (cached) → launch the
//! bioconda solve as a DETACHED background job → return promptly → pre-wire the env bin onto PATH.
//! The (minutes-long) bioconda solve itself is delegated to the background job (its mechanism is
//! covered by bench_j; a micromamba create completing is shown manually). Direct-kernel, no API.
//!   cargo run -p axiom-orchestrate --example bench_m
use axiom_orchestrate::kernel::PyKernel;
use std::time::{Duration, Instant};

// Mirrors coding::CONDA_INSTALL (id=condam, pkgs=["tqdm"]).
const LAUNCH: &str = r#"import os, sys, subprocess, platform, tarfile, io, requests
ROOT = os.path.expanduser('~/.axiom3'); MM = os.path.join(ROOT, 'bin', 'micromamba'); ENV = os.path.join(ROOT, 'bioenv')
os.makedirs(os.path.join(ROOT, 'bin'), exist_ok=True); os.makedirs('.axiom_jobs', exist_ok=True)
if not os.path.exists(MM):
    _s = platform.system().lower(); _m = platform.machine().lower()
    _key = ('osx-arm64' if _m in ('arm64','aarch64') else 'osx-64') if _s == 'darwin' else (('linux-aarch64' if _m in ('aarch64','arm64') else 'linux-64') if _s == 'linux' else None)
    _d = requests.get('https://micro.mamba.pm/api/micromamba/%s/latest' % _key, timeout=120).content
    with tarfile.open(fileobj=io.BytesIO(_d), mode='r:bz2') as _tf:
        _mem = next(x for x in _tf.getmembers() if x.name.endswith('bin/micromamba'))
        _mem.name = os.path.basename(_mem.name); _tf.extract(_mem, os.path.join(ROOT, 'bin'))
    os.chmod(MM, 0o755)
_sub = 'install' if os.path.isdir(ENV) else 'create'
_pkgs = ['tqdm']
_cmd = [MM, _sub, '-y', '-p', ENV, '-c', 'conda-forge', '-c', 'bioconda'] + _pkgs
_wrap = ("import subprocess, sys\n" "r = subprocess.run(%r, capture_output=True, text=True)\n" "open('.axiom_jobs/condam.status', 'w').write('done' if r.returncode == 0 else 'failed')\n") % (_cmd,)
open('.axiom_jobs/condam.py', 'w').write(_wrap)
_p = subprocess.Popen([sys.executable, '.axiom_jobs/condam.py'], stdout=open('.axiom_jobs/condam.log', 'w'), stderr=subprocess.STDOUT, start_new_session=True)
_eb = os.path.join(ENV, 'bin'); os.environ['PATH'] = _eb + os.pathsep + os.environ.get('PATH', '')
print('micromamba ready; installing', _pkgs, 'as background job condam (pid', _p.pid, ')')
print('PATHHAS', _eb in os.environ['PATH'])
"#;

fn main() {
    let venv = std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
        .join("Library/Application Support/com.axiom3.desktop/notebook/.venv/bin/python");
    let py = if venv.exists() { venv.to_string_lossy().into_owned() } else { "python3".into() };
    let dir = std::env::temp_dir().join(format!("axiom-bench-m-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    println!("=== #3 验证:conda_install 同步机制(micromamba bootstrap + 后台作业 + PATH) ===");
    let k = PyKernel::start(&py, &dir).expect("kernel");
    let t0 = Instant::now();
    let cell = k.exec(LAUNCH, Duration::from_secs(120)).expect("launch cell");
    let elapsed = t0.elapsed().as_secs_f64();
    let out = cell.stdout;
    println!("{}", out.trim());
    println!("(返回耗时 {:.1}s)", elapsed);

    let mm = std::path::PathBuf::from(std::env::var_os("HOME").unwrap()).join(".axiom3/bin/micromamba");
    let job_py = dir.join(".axiom_jobs/condam.py");
    let micromamba_ready = mm.exists();
    let job_launched = job_py.exists() && out.contains("background job condam");
    let path_wired = out.contains("PATHHAS True");
    let prompt_returned_fast = elapsed < 90.0; // bootstrap only; the solve is detached
    println!("\n=== micromamba 就绪:{} · 后台作业已起:{} · PATH 预挂:{} · 快速返回(<90s):{} ===",
        micromamba_ready, job_launched, path_wired, prompt_returned_fast);
    // Give the detached job a moment; it will keep solving bioconda in the background regardless.
    let _ = std::fs::remove_dir_all(&dir);
    let ok = micromamba_ready && job_launched && path_wired && prompt_returned_fast;
    println!("\n=== 判定: 二进制工具安装机制(bootstrap→后台→PATH)= {} ===", if ok { "PASS ✓" } else { "未达标" });
    std::process::exit(if ok { 0 } else { 1 });
}
