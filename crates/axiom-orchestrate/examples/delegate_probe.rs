//! Live check of the `delegate` sub-workflow: decompose → sub-agents → synthesize.
//!   ZHIPU_API_KEY=... AXIOM_MODEL=glm-5.2 cargo run -p axiom-orchestrate --example delegate_probe
use axiom_orchestrate::coding::{run_delegate, ProviderModel};
use axiom_runtime::supervisor::Cancel;

#[tokio::main]
async fn main() {
    let provider = std::env::var("AXIOM_PROVIDER").unwrap_or_else(|_| "zhipu".into());
    let model_id = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "glm-5.2".into());
    let m = ProviderModel { provider, model: model_id.clone(), timeout_s: 120 };
    let task = std::env::var("AXIOM_TASK").unwrap_or_else(|_| {
        "系统梳理'衰老细胞如何逃避免疫清除'的各种已知与假说机制,并权衡哪个方向最有前景。".into()
    });
    println!("=== delegate (model={model_id}) ===\n=== task: {task}\n");
    let out = run_delegate(&m, &task, &Cancel::new()).await;
    println!("{out}");
}
