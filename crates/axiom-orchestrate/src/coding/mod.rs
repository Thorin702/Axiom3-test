//! The coding agent: a bounded tool-use loop over a fail-closed [`Sandbox`]. Uses a
//! strict-JSON action envelope over the text seam (native tool-use / token usage is the
//! `complete_blocks` follow-up). In read-only mode it reads/lists/searches and proposes;
//! in read-write mode it can also write_file and run build/test commands, so it can
//! implement a change, run the tests, and iterate to green (verified live in
//! examples/coding_live.rs). The sandbox keeps every path inside the workspace and runs
//! commands shell-free against a mode-gated allowlist.
pub mod sandbox;

use crate::control::Pause;
use crate::kernel::PyKernel;
use axiom_providers::message::{ContentBlock, Message, Request};
use axiom_runtime::supervisor::{is_retryable_error, retry_async, Cancel, RetryPolicy};
use axiom_runtime::{complete_with, Provider};
use sandbox::{Sandbox, SandboxMode};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

pub struct CodingBudget {
    pub max_steps: u32,
    pub per_call_timeout_s: u64,
    pub cmd_timeout_s: u64,
}
impl Default for CodingBudget {
    fn default() -> Self {
        Self { max_steps: 24, per_call_timeout_s: 90, cmd_timeout_s: 45 }
    }
}

#[derive(Clone, Debug)]
pub struct CodingStepRec {
    pub n: u32,
    pub tool: String,
    pub detail: String,
    pub observation: String,
    /// The FULL tool input (complete python source / command / path / query), for an
    /// auditable execution log — `detail` is only a short label. Empty when not applicable.
    pub input: String,
    /// The model's free-text reasoning around the action (its "thinking" this turn).
    pub reasoning: String,
    /// Matplotlib figures (base64 PNG) captured from a `python` step this turn — computed
    /// from real data, never asserted. Empty for non-python steps.
    pub figures: Vec<String>,
}

pub struct CodingResult {
    pub status: String, // "done" | "failed" | "cancelled"
    pub summary: String,
    pub steps: u32,
    /// All figures the agent computed across the run, in order — surfaced into the artifact.
    pub figures: Vec<String>,
}

/// The model seam for the loop — abstracted so the loop is testable offline.
pub trait CodingModel: Send + Sync {
    fn complete(
        &self,
        system: &str,
        user: &str,
        cancel: &Cancel,
    ) -> impl std::future::Future<Output = anyhow::Result<String>> + Send;
}

/// Real model: one text completion per turn (history is flattened into `user` since the
/// text seam keeps no history), wrapped in the fleet's retry + timeout template.
pub struct ProviderModel {
    pub provider: String,
    pub model: String,
    pub timeout_s: u64,
}
impl CodingModel for ProviderModel {
    fn complete(
        &self,
        system: &str,
        user: &str,
        _cancel: &Cancel,
    ) -> impl std::future::Future<Output = anyhow::Result<String>> + Send {
        let provider_s = self.provider.clone();
        let model = self.model.clone();
        let system = system.to_string();
        let user = user.to_string();
        let timeout_s = self.timeout_s;
        async move {
            let provider = Provider::parse(&provider_s)
                .ok_or_else(|| anyhow::anyhow!("unknown provider: {provider_s}"))?;
            let req = Request {
                system: Some(system),
                messages: vec![Message::user(vec![ContentBlock::text(user)])],
                model,
                max_tokens: None,
                reasoning_effort: None,
                tools: Vec::new(),
            };
            retry_async(&RetryPolicy::default(), is_retryable_error, || async {
                tokio::time::timeout(Duration::from_secs(timeout_s), complete_with(provider, &req))
                    .await
                    .map_err(|_| anyhow::anyhow!("model timed out after {timeout_s}s"))?
            })
            .await
        }
    }
}

fn system_prompt(mode: SandboxMode, has_kernel: bool) -> String {
    let mut s = String::from(
        "You are a coding agent working inside a sandboxed workspace. Each turn respond with \
         EXACTLY ONE JSON object and nothing else — one of:\n\
         {\"tool\":\"read_file\",\"path\":\"<relative path>\"}\n\
         {\"tool\":\"list_dir\",\"path\":\"<relative path>\"}\n\
         {\"tool\":\"search\",\"query\":\"<text to grep for>\"}\n",
    );
    if mode == SandboxMode::ReadWrite {
        s.push_str(
            "{\"tool\":\"write_file\",\"path\":\"<relative path>\",\"content\":\"<COMPLETE new file contents>\"}\n\
             {\"tool\":\"run\",\"cmd\":[\"prog\",\"arg\",...]}   (build/test, e.g. [\"cargo\",\"test\"] or [\"pytest\",\"-q\"])\n\
             {\"final\":\"<what you changed and the test result>\"}\n\
             You CAN create/overwrite files and run build/test commands. Make the change, RUN the \
             tests, read failures, and iterate until they pass — THEN finish. write_file replaces the \
             WHOLE file, so include its complete contents.",
        );
    } else {
        s.push_str(
            "{\"tool\":\"run\",\"cmd\":[\"prog\",\"arg\",...]}   (read-only allowlisted commands only)\n\
             {\"final\":\"<your findings and proposed changes>\"}\n\
             You CANNOT write files or run build/test commands here — describe any changes in your final answer.",
        );
    }
    if has_kernel {
        s.push_str(
            "\n{\"tool\":\"python\",\"code\":\"<python source>\"}   (a PERSISTENT Python kernel: \
             variables/dataframes/models stay in memory across python calls, so build up an analysis \
             step by step. stdout and the last expression's value come back to you; any matplotlib \
             figure you create is CAPTURED into the report — do `import matplotlib.pyplot as plt` and \
             draw with it. numpy/pandas/matplotlib are available.)\n\
             Use python to COMPUTE results and figures from real data — never state a number, table, \
             or trend you did not actually compute. In your final answer, describe what the computed \
             results and figures show; the figures themselves are attached automatically.\n\
             For a crisp, interactive figure in the report, you may instead emit a NATIVE figure block \
             in your final answer, filled with values you computed in python. Use these EXACT shapes: \
             ```chart bar = {\"type\":\"bar\",\"title\":..,\"xlabel\":..,\"ylabel\":..,\"data\":[{\"label\":\"Jan\",\"value\":12}, …]}; \
             ```chart scatter = {\"type\":\"scatter\",...,\"points\":[{\"x\":1,\"y\":2,\"group\":?}, …]}; \
             ```chart line = {\"type\":\"line\",...,\"series\":[{\"name\":..,\"points\":[[x,y], …]}]}. \
             Other blocks: ```forest {\"rows\":[{\"label\":..,\"estimate\":..,\"lo\":..,\"hi\":..}]}, \
             ```heatmap {\"rows\":[..],\"cols\":[..],\"values\":[[..]],\"scale\":\"sequential|diverging\"}, \
             ```dist {\"groups\":[{\"label\":..,\"values\":[..]}]}. Use real computed numbers only — \
             never fabricated ones, and match the shapes exactly.",
        );
    }
    s.push_str(" Be efficient; do not repeat a read you already did.");
    s
}

/// Run the loop. `on_step` is called after each tool execution (the shell streams it as
/// an event). Cancellation is checked every turn (cooperative — an in-flight command
/// still finishes or hits its timeout).
pub async fn run_coding_agent(
    goal: &str,
    sandbox: &Sandbox,
    model: &impl CodingModel,
    cancel: &Cancel,
    pause: &Pause,
    budget: &CodingBudget,
    kernel: Option<Arc<PyKernel>>,
    mut on_step: impl FnMut(CodingStepRec),
) -> CodingResult {
    let sys = system_prompt(sandbox.mode(), kernel.is_some());
    let mut transcript = format!("GOAL:\n{goal}\n\nWorkspace root: {}\n", sandbox.root().display());
    let mut ran_command = false; // did the agent run a build/test command this run?
    let mut nudged = false; // pushed back once for finishing without verifying
    let mut all_figures: Vec<String> = Vec::new(); // computed figures across the whole run
    for n in 1..=budget.max_steps {
        // Cooperative pause: wait here between turns while paused (cancel still wins).
        while pause.is_paused() && !cancel.is_cancelled() {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        if cancel.is_cancelled() {
            return CodingResult { status: "cancelled".into(), summary: "cancelled".into(), steps: n - 1, figures: all_figures };
        }
        let user = format!("{transcript}\nNext action (one JSON object only):");
        let reply = match model.complete(&sys, &user, cancel).await {
            Ok(r) => r,
            Err(e) => return CodingResult { status: "failed".into(), summary: format!("model error: {e}"), steps: n - 1, figures: all_figures },
        };
        let json = extract_json(&reply);
        // The model's reasoning = its reply minus the JSON action it chose this turn.
        let reasoning = match &json {
            Some(j) => reply.replacen(j, "", 1).trim().to_string(),
            None => String::new(),
        };
        let action = json.unwrap_or_else(|| reply.trim().to_string());
        let v: Value = match serde_json::from_str(&action) {
            Ok(v) => v,
            Err(_) => return CodingResult { status: "done".into(), summary: reply.trim().to_string(), steps: n, figures: all_figures },
        };
        if let Some(f) = v.get("final").and_then(|x| x.as_str()) {
            // In write mode, don't accept "done" until the agent has actually RUN a
            // build/test command at least once — nudge it once, like Codex verifying.
            if sandbox.mode() == SandboxMode::ReadWrite && !ran_command && !nudged {
                nudged = true;
                transcript.push_str(
                    "\n[system] You haven't run any build/test command yet. Before finishing, RUN \
                     the tests (e.g. `python3 check.py`, `pytest -q`, or `cargo test`) to confirm \
                     your change actually works — then finish.\n",
                );
                continue;
            }
            return CodingResult { status: "done".into(), summary: f.to_string(), steps: n, figures: all_figures };
        }
        let tool = v.get("tool").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let mut step_figures: Vec<String> = Vec::new(); // figures this turn (python only)
        let (detail, observation) = match tool.as_str() {
            "read_file" => {
                let p = arg_str(&v, "path");
                (p.to_string(), sandbox.read_file(p).unwrap_or_else(|e| format!("ERROR: {e}")))
            }
            "list_dir" => {
                let p = arg_str(&v, "path");
                (p.to_string(), sandbox.list_dir(p).unwrap_or_else(|e| format!("ERROR: {e}")))
            }
            "write_file" => {
                let p = arg_str(&v, "path").to_string();
                let content = arg_str(&v, "content");
                let obs = match sandbox.write_file(&p, content) {
                    Ok(()) => format!("wrote {p} ({} bytes)", content.len()),
                    Err(e) => format!("ERROR: {e}"),
                };
                (p, obs)
            }
            "search" => {
                let q = arg_str(&v, "query").to_string();
                let argv = vec!["rg".into(), "-n".into(), "--max-count".into(), "40".into(), q.clone()];
                let obs = match sandbox.run_command(&argv, Duration::from_secs(budget.cmd_timeout_s)).await {
                    Ok(r) => if r.output.trim().is_empty() { "(no matches)".into() } else { r.output },
                    Err(e) => format!("ERROR: {e}"),
                };
                (q, obs)
            }
            "run" => {
                ran_command = true;
                let argv: Vec<String> = v
                    .get("cmd")
                    .and_then(|x| x.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                let label = argv.join(" ");
                let obs = match sandbox.run_command(&argv, Duration::from_secs(budget.cmd_timeout_s)).await {
                    Ok(r) => format!("(exit {:?})\n{}", r.code, r.output),
                    Err(e) => format!("ERROR: {e}"),
                };
                (label, obs)
            }
            "python" => match &kernel {
                Some(k) => {
                    ran_command = true; // executing code counts as "ran something" (nudge)
                    let code = arg_str(&v, "code").to_string();
                    let label: String = code.lines().next().unwrap_or("").chars().take(60).collect();
                    let k = k.clone();
                    // Blocking kernel call off the async worker; generous timeout for analysis.
                    let timeout = Duration::from_secs(budget.cmd_timeout_s.max(60));
                    let obs = match tokio::task::spawn_blocking(move || k.exec(&code, timeout)).await {
                        Ok(Ok(cell)) => {
                            step_figures = cell.figures;
                            let mut o = cell.stdout;
                            if let Some(err) = cell.error {
                                if !o.is_empty() && !o.ends_with('\n') { o.push('\n'); }
                                o.push_str(&err);
                            }
                            if !step_figures.is_empty() {
                                o.push_str(&format!("\n[{} figure(s) captured into the report]", step_figures.len()));
                            }
                            if o.trim().is_empty() { o = "(ran; no output)".into(); }
                            o
                        }
                        Ok(Err(e)) => format!("ERROR: {e}"),
                        Err(e) => format!("ERROR: python kernel task failed: {e}"),
                    };
                    (label, obs)
                }
                None => ("python".to_string(), "ERROR: python tool not available in this run".to_string()),
            },
            other => (other.to_string(), format!("ERROR: unknown tool '{other}'")),
        };
        all_figures.extend(step_figures.iter().cloned());
        let obs = truncate(&observation, 4000);
        // The full tool input, for the execution log (detail is only a short label).
        let input = match tool.as_str() {
            "python" => arg_str(&v, "code").to_string(),
            "read_file" | "list_dir" | "write_file" => arg_str(&v, "path").to_string(),
            "search" => arg_str(&v, "query").to_string(),
            _ => detail.clone(), // "run": the joined argv
        };
        on_step(CodingStepRec { n, tool: tool.clone(), detail: detail.clone(), observation: obs.clone(), input, reasoning: reasoning.clone(), figures: step_figures });
        let think = if reasoning.is_empty() { String::new() } else { format!("THINKING: {}\n", truncate(&reasoning, 500)) };
        transcript.push_str(&format!("\n--- step {n}: {tool} {detail} ---\n{think}{obs}\n"));
    }
    CodingResult {
        status: "done".into(),
        summary: format!("Reached the {}-step budget without finishing. Latest context:\n{}", budget.max_steps, truncate(&transcript, 2000)),
        steps: budget.max_steps,
        figures: all_figures,
    }
}

/// Extract the first balanced top-level `{...}` object from a reply (tolerate fences /
/// prose around it).
fn extract_json(s: &str) -> Option<String> {
    let start = s.find('{')?;
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for i in start..s.len() {
        let c = bytes[i] as char;
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(s[start..=i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn arg_str<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("")
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let kept: String = s.chars().take(n).collect();
        format!("{kept}\n…[truncated]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Model that returns canned replies in order.
    struct ScriptModel {
        replies: Mutex<Vec<String>>,
    }
    impl CodingModel for ScriptModel {
        fn complete(
            &self,
            _system: &str,
            _user: &str,
            _cancel: &Cancel,
        ) -> impl std::future::Future<Output = anyhow::Result<String>> + Send {
            let next = {
                let mut r = self.replies.lock().unwrap();
                if r.is_empty() {
                    None
                } else {
                    Some(r.remove(0))
                }
            };
            async move { next.ok_or_else(|| anyhow::anyhow!("script exhausted")) }
        }
    }

    fn tmp(label: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("axiom-coding-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("main.rs"), "fn main() { println!(\"hi\"); }").unwrap();
        d
    }

    #[tokio::test]
    async fn loop_reads_then_finalizes() {
        let root = tmp("reads");
        let sb = Sandbox::new(&root).unwrap();
        let model = ScriptModel {
            replies: Mutex::new(vec![
                r#"{"tool":"list_dir","path":"."}"#.into(),
                r#"{"tool":"read_file","path":"main.rs"}"#.into(),
                r#"{"final":"main.rs prints hi; no issues."}"#.into(),
            ]),
        };
        let mut steps = 0;
        let res = run_coding_agent("review main.rs", &sb, &model, &Cancel::new(), &Pause::new(), &CodingBudget::default(), None, |_| steps += 1).await;
        assert_eq!(res.status, "done");
        assert!(res.summary.contains("prints hi"));
        assert_eq!(steps, 2); // list_dir + read_file (final isn't a tool step)
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn rw_loop_writes_a_file() {
        let root = tmp("rw");
        let sb = Sandbox::with_mode(&root, SandboxMode::ReadWrite).unwrap();
        let model = ScriptModel {
            replies: Mutex::new(vec![
                r#"{"tool":"write_file","path":"hello.txt","content":"hi there"}"#.into(),
                r#"{"tool":"run","cmd":["cat","hello.txt"]}"#.into(), // verifies → satisfies the "run before final" nudge
                r#"{"final":"created hello.txt"}"#.into(),
            ]),
        };
        let res = run_coding_agent("make hello.txt", &sb, &model, &Cancel::new(), &Pause::new(), &CodingBudget::default(), None, |_| {}).await;
        assert_eq!(res.status, "done");
        assert_eq!(std::fs::read_to_string(root.join("hello.txt")).unwrap(), "hi there");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn readonly_loop_cannot_write() {
        let root = tmp("ro");
        let sb = Sandbox::new(&root).unwrap(); // read-only
        let model = ScriptModel {
            replies: Mutex::new(vec![
                r#"{"tool":"write_file","path":"x.txt","content":"nope"}"#.into(),
                r#"{"final":"could not write"}"#.into(),
            ]),
        };
        let res = run_coding_agent("try to write", &sb, &model, &Cancel::new(), &Pause::new(), &CodingBudget::default(), None, |_| {}).await;
        assert_eq!(res.status, "done");
        assert!(!root.join("x.txt").exists(), "read-only sandbox must not create files");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn cancel_stops_the_loop() {
        let root = tmp("cancel");
        let sb = Sandbox::new(&root).unwrap();
        let model = ScriptModel { replies: Mutex::new(vec![r#"{"tool":"list_dir","path":"."}"#.into()]) };
        let cancel = Cancel::new();
        cancel.cancel();
        let res = run_coding_agent("x", &sb, &model, &cancel, &Pause::new(), &CodingBudget::default(), None, |_| {}).await;
        assert_eq!(res.status, "cancelled");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn python_tool_runs_in_persistent_kernel() {
        // Needs a real python3; skip cleanly where absent (CI images without it).
        let py = match ["python3", "python"].iter().find(|p| {
            std::process::Command::new(p).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
        }) {
            Some(p) => p.to_string(),
            None => return,
        };
        let root = tmp("py");
        let sb = Sandbox::with_mode(&root, SandboxMode::ReadWrite).unwrap();
        let kernel = Arc::new(crate::kernel::PyKernel::start(&py, &root).unwrap());
        let model = ScriptModel {
            replies: Mutex::new(vec![
                r#"{"tool":"python","code":"x = 6 * 7\nprint(x)"}"#.into(),
                r#"{"tool":"python","code":"print(x + 1)"}"#.into(), // x persists across calls
                r#"{"final":"computed 42 then 43"}"#.into(),
            ]),
        };
        let mut obs: Vec<String> = Vec::new();
        let mut inputs: Vec<String> = Vec::new();
        let res = run_coding_agent(
            "compute", &sb, &model, &Cancel::new(), &Pause::new(), &CodingBudget::default(),
            Some(kernel), |s| { obs.push(s.observation); inputs.push(s.input); },
        )
        .await;
        assert_eq!(res.status, "done", "summary: {}", res.summary);
        assert!(obs.len() >= 2, "expected two python steps, got {obs:?}");
        assert!(obs[0].contains("42"), "first python step stdout: {}", obs[0]);
        assert!(obs[1].contains("43"), "kernel state must persist across python calls: {}", obs[1]);
        // The execution log needs the FULL code, not just the first line.
        assert!(inputs[0].contains("x = 6 * 7") && inputs[0].contains("print(x)"), "input must be the full code, got: {}", inputs[0]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn extract_json_handles_fences_and_braces() {
        assert_eq!(extract_json("```json\n{\"a\":1}\n```").as_deref(), Some("{\"a\":1}"));
        assert_eq!(extract_json("sure: {\"final\":\"a {nested} brace\"} done").as_deref(), Some("{\"final\":\"a {nested} brace\"}"));
        assert_eq!(extract_json("no json here"), None);
    }
}
