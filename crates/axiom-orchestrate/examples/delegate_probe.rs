//! Live check of `delegate`: reasoning fan-out (run_delegate) OR tool-using sub-agents (delegate_tooled).
//!   ZHIPU_API_KEY=... AXIOM_MODEL=glm-5.2 [AXIOM_TOOLED=1] cargo run -p axiom-orchestrate --example delegate_probe
use axiom_orchestrate::coding::sandbox::{Sandbox, SandboxMode};
use axiom_orchestrate::coding::{delegate_tooled, run_delegate, CodingBudget, ProviderModel};
use axiom_orchestrate::kernel::PyKernel;
use axiom_runtime::supervisor::Cancel;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let provider = std::env::var("AXIOM_PROVIDER").unwrap_or_else(|_| "zhipu".into());
    let model_id = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "glm-5.2".into());
    let m = ProviderModel { provider, model: model_id.clone(), timeout_s: 150 };
    let cancel = Cancel::new();

    if std::env::var("AXIOM_TOOLED").is_ok() {
        // TOOL-USING sub-agents (each runs the full litsearch/web/python loop). Small budget for speed.
        let venv = dirs_data().join("com.axiom3.desktop/notebook/.venv/bin/python");
        let py = if venv.exists() { venv.to_string_lossy().into_owned() } else { "python3".into() };
        let dir = std::env::temp_dir().join(format!("axiom-delegate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sb = Sandbox::with_mode(&dir, SandboxMode::ReadWrite).unwrap();
        let kernel = Some(Arc::new(PyKernel::start(&py, &dir).expect("kernel")));
        let sub_budget = CodingBudget { max_steps: 6, per_call_timeout_s: 150, cmd_timeout_s: 120, cell_timeout_s: 300 };
        let task = std::env::var("AXIOM_TASK").unwrap_or_else(|_| "查证:端粒酶激活是否是衰老细胞免疫逃逸的一个已被报道的机制?给出结论与依据。".into());
        println!("=== delegate_tooled (model={model_id}) ===\n=== task: {task}\n");
        let mut on_life = |_: axiom_orchestrate::coding::CodingStepRec| {};
        let (obs, figs, merged) = delegate_tooled(&m, &sb, &kernel, &sub_budget, &cancel, &task, 0, &mut on_life).await;
        println!("{obs}\n\n[figures produced by sub-agents: {}] [merged sources: {}]", figs.len(), merged.len());
        let _ = std::fs::remove_dir_all(&dir);
    } else {
        let task = std::env::var("AXIOM_TASK").unwrap_or_else(|_| "系统梳理'衰老细胞如何逃避免疫清除'的各种机制,并权衡哪个方向最有前景。".into());
        println!("=== run_delegate (reasoning, model={model_id}) ===\n=== task: {task}\n");
        println!("{}", run_delegate(&m, &task, &cancel).await);
    }
}

fn dirs_data() -> std::path::PathBuf {
    std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Library/Application Support")).unwrap_or_else(|| std::path::PathBuf::from("."))
}
