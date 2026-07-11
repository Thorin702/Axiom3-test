//! Verifies P2 checkpoint/restore: kernel state pickled to disk survives a FULL kernel restart.
//! Direct kernel test (no model/API) — deterministic proof of the mechanism the checkpoint/restore
//! tools run.
//!   cargo run -p axiom-orchestrate --example bench_k
use axiom_orchestrate::kernel::PyKernel;
use std::time::Duration;

const CHECKPOINT: &str = "import pickle as _pk\n_ck = {}\nfor _k, _v in list(globals().items()):\n    if _k.startswith('_'): continue\n    try:\n        _pk.dumps(_v)\n        _ck[_k] = _v\n    except Exception:\n        pass\n_pk.dump(_ck, open('.axiom_ckpt.pkl', 'wb'))\nprint('checkpointed', len(_ck), 'objects:', sorted(_ck.keys()))\n";
const RESTORE: &str = "import pickle as _pk, os\nif os.path.exists('.axiom_ckpt.pkl'):\n    _ck = _pk.load(open('.axiom_ckpt.pkl', 'rb'))\n    globals().update(_ck)\n    print('restored', len(_ck), 'objects:', sorted(_ck.keys()))\nelse:\n    print('no checkpoint found (.axiom_ckpt.pkl missing)')\n";

fn main() {
    let venv = std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
        .join("Library/Application Support/com.axiom3.desktop/notebook/.venv/bin/python");
    let py = if venv.exists() { venv.to_string_lossy().into_owned() } else { "python3".into() };
    let dir = std::env::temp_dir().join(format!("axiom-bench-k-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let to = Duration::from_secs(60);

    println!("=== P2 验证:checkpoint 跨 kernel 重启保命 ===");
    // Kernel 1: build up state, then checkpoint.
    let k1 = PyKernel::start(&py, &dir).expect("k1");
    k1.exec("x = 6 * 7\ny = [1, 2, 3]\nimport pandas as pd\ndf = pd.DataFrame({'a':[10,20,30]})\nname='axiom'", to).unwrap();
    let ck = k1.exec(CHECKPOINT, to).unwrap();
    println!("  [k1] {}", ck.stdout.trim());
    drop(k1); // kernel gone — all in-memory state lost

    // Kernel 2: fresh interpreter (state reset), confirm loss, then restore.
    let k2 = PyKernel::start(&py, &dir).expect("k2");
    let before = k2.exec("print('x' in dir(), 'df' in dir())", to).unwrap();
    println!("  [k2 before restore] vars present? {}", before.stdout.trim());
    let rs = k2.exec(RESTORE, to).unwrap();
    println!("  [k2 restore] {}", rs.stdout.trim());
    let after = k2.exec("print(x, y, df['a'].sum(), name)", to).unwrap();
    let out = after.stdout.trim().to_string();
    println!("  [k2 after restore] x,y,df.a.sum(),name = {}", out);

    // 42, [1,2,3], 60, axiom — all recovered from a brand-new kernel.
    let ok = before.stdout.contains("False False")
        && out.contains("42")
        && out.contains("[1, 2, 3]")
        && out.contains("60")
        && out.contains("axiom");
    println!("\n=== 判定: 全新 kernel 从 checkpoint 恢复变量/DataFrame = {} ===", if ok { "PASS ✓" } else { "未达标" });
    let _ = std::fs::remove_dir_all(&dir);
    std::process::exit(if ok { 0 } else { 1 });
}
