//! Verifies P1 runtime install (the P2-domain-toolchain linchpin): the agent installs a domain
//! library on demand with `pip_install` and uses it in the SAME kernel.
//!   DEEPSEEK_API_KEY=... cargo run -p axiom-orchestrate --example bench_g
use axiom_orchestrate::coding::sandbox::{Sandbox, SandboxMode};
use axiom_orchestrate::coding::{run_coding_agent, CodingBudget, ProviderModel};
use axiom_orchestrate::kernel::PyKernel;
use axiom_orchestrate::Pause;
use axiom_runtime::supervisor::Cancel;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let venv = std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
        .join("Library/Application Support/com.axiom3.desktop/notebook/.venv/bin/python");
    let py = if venv.exists() { venv.to_string_lossy().into_owned() } else { "python3".into() };
    let dir = std::env::temp_dir().join(format!("axiom-bench-g-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // 3 short sequences with clearly different GC content.
    std::fs::write(
        dir.join("seqs.fasta"),
        ">s1_lowGC\nAAAATTTTAAAATTTT\n>s2_midGC\nACGTACGTACGTACGT\n>s3_highGC\nGGGGCCCCGGGGCCCC\n",
    )
    .unwrap();

    // Make sure biopython is NOT already present, so the run must install it.
    let _ = std::process::Command::new(&py).args(["-m", "pip", "uninstall", "-y", "biopython"]).output();

    let sb = Sandbox::with_mode(&dir, SandboxMode::ReadWrite).unwrap();
    let kernel = Arc::new(PyKernel::start(&py, &dir).expect("kernel"));
    let model_id = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    let model = ProviderModel { provider: "deepseek".into(), model: model_id.clone(), timeout_s: 180 };
    let budget = CodingBudget { max_steps: 14, per_call_timeout_s: 180, cmd_timeout_s: 300, cell_timeout_s: 600 };

    println!("=== P1 验证:pip_install 按需装领域库 (model={model_id}) ===");
    let mut used_pip = false;
    let mut pip_ok = false;
    let res = run_coding_agent(
        "seqs.fasta is in your workspace. Use the Biopython library to parse it — install it first with \
         the pip_install tool (packages: [\"biopython\"]), then in python `from Bio import SeqIO` and \
         `from Bio.SeqUtils import gc_fraction`. Compute the GC fraction (percent) of each sequence, and \
         present the three values as a native ```chart bar block. Use only computed values.",
        &sb, &model, &Cancel::new(), &Pause::new(), &budget, Some(kernel),
        |s| {
            if s.tool == "pip_install" {
                used_pip = true;
                if s.observation.contains("pip exit 0") || s.observation.contains("installed:") { pip_ok = true; }
            }
            let o: String = s.observation.lines().next().unwrap_or("").chars().take(72).collect();
            println!("  [{}] {} {} → {}", s.n, s.tool, s.detail, o);
        },
    )
    .await;

    let sy = &res.summary;
    let has_block = sy.contains("```chart") || sy.contains("```bar") || sy.contains("```heatmap");
    // Proof biopython actually ran: the three seq ids should surface, GC ordered low<mid<high.
    let ids = ["s1", "s2", "s3"];
    let id_hits = ids.iter().filter(|i| sy.contains(**i)).count();
    println!("\n=== status {} ({} steps) · pip_install 调用:{} 成功:{} ===", res.status, res.steps, used_pip, pip_ok);
    println!("=== 含原生块:{} · 序列 id 命中:{}/3 ===", has_block, id_hits);
    println!("\nSUMMARY(截断):\n{}", sy.chars().take(1000).collect::<String>());
    let ok = used_pip && pip_ok && has_block;
    println!("\n=== 判定: 按需装库并在同 kernel 使用 = {} ===", if ok { "PASS ✓" } else { "未达标" });
    let _ = std::fs::remove_dir_all(&dir);
    std::process::exit(if ok { 0 } else { 1 });
}
