//! axiom-orchestrate — the dynamic multi-agent orchestration engine.
//!
//! A run is: decompose a goal into sub-agents → fan them out concurrently (the scheduler
//! is `axiom_runtime::supervisor::run_batch`, reused verbatim: bounded concurrency,
//! cancel-at-entry, `catch_unwind` panic isolation) → stream each agent's lifecycle on
//! the `EventSink` → fold the outputs. Coding becomes one more `AgentStep` impl; a DAG
//! scheduler degenerates back to `run_batch` for a flat plan, so nothing here breaks
//! when it lands.
//!
//! RED LINE: this crate produces FREE-FORM work and must NOT depend on axiom-core (the
//! verdict types). The truth-checking tier is a separate engine; this is orchestration.
pub mod coding;
pub mod control;
pub mod events;
pub mod kernel;
pub mod model;
pub mod search;
pub mod step;
pub mod supervisor;
pub mod synthesize;

pub use control::Pause;
pub use search::{orchestrate_search, ModelArm, SearchConfig};

use events::{now_ms, EventSink, RunEventBody, SeqEmitter};
use model::{Kind, Plan, Run, RunStatus, StepSpec, StepState, StepStatus};
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use step::{AgentStep, ProviderStep, StepInput};
use supervisor::{Decomposer, LlmDecomposer};

use axiom_runtime::supervisor::{run_batch, Cancel};

pub struct OrchestrateOptions {
    pub run_id: String,
    /// Provider/model used for synthesis (steps carry their own from the plan).
    pub provider: String,
    pub model: String,
    pub concurrency: usize,
}

/// Run a workflow goal end-to-end, streaming events through `sink`, and return the
/// fully-populated terminal `Run`.
pub async fn orchestrate(
    goal: &str,
    opts: &OrchestrateOptions,
    decomposer: &impl Decomposer,
    step_exec: &impl AgentStep,
    sink: Arc<dyn EventSink>,
    cancel: &Cancel,
) -> Run {
    let em = Arc::new(SeqEmitter::new(sink, opts.run_id.clone()));
    let mut run = Run {
        id: opts.run_id.clone(),
        kind: Kind::Workflow,
        goal: goal.to_string(),
        status: RunStatus::Planning,
        steps: Vec::new(),
        synthesis: None,
        review: None,
        created_ms: now_ms(),
    };

    em.emit(RunEventBody::RunStarted { task_kind: "workflow".into(), title: title_of(goal), cancellable: true });

    // ── plan ──
    let plan: Plan = match decomposer.plan(goal, &opts.run_id, cancel).await {
        Ok(p) => p,
        Err(e) => return fail(run, &em, e.to_string()),
    };
    if let Err(e) = plan.validate_acyclic() {
        return fail(run, &em, e);
    }
    let total = plan.steps.len();
    run.steps = plan.steps.iter().cloned().map(StepState::new).collect();
    em.emit(RunEventBody::PlanReady {
        phases: vec!["planning".into(), "executing".into(), "verifying".into(), "synthesizing".into(), "done".into()],
        planned_agents: total,
        notes: vec![],
    });

    if cancel.is_cancelled() {
        run.status = RunStatus::Cancelled;
        em.emit(RunEventBody::RunFinished { status: "cancelled".into(), agents_total: total, agents_ok: 0, agents_err: 0, error: None });
        return run;
    }

    // ── execute: flat fan-out, scheduler == run_batch ──
    run.status = RunStatus::Running;
    em.emit(RunEventBody::PhaseChanged { phase: "executing".into(), index: 1, total: 5 });

    let em_task = em.clone();
    let em_prog = em.clone();
    let specs = plan.steps.clone();
    // The task body returns (StepId, Result<StepOutcome>) so every result correlates to
    // its input — this is the one fix run_batch needs (it is otherwise lossy/unordered).
    let results = run_batch(
        specs,
        opts.concurrency.max(1),
        cancel,
        move |spec| {
            let em = em_task.clone();
            async move {
                em.emit(RunEventBody::AgentSpawned {
                    agent_id: spec.id.clone(),
                    role: spec.role.clone(),
                    model: spec.model.clone(),
                    vendor: spec.provider.clone(),
                    phase: "executing".into(),
                });
                let id = spec.id.clone();
                let input = StepInput { spec, dep_outputs: Vec::new() };
                let res = step_exec.run(input, cancel).await;
                let (outcome, ti, to, err, summary) = match &res {
                    Ok(o) => ("ok", o.tokens_in, o.tokens_out, None, Some(truncate(&o.output, 120))),
                    Err(e) => ("err", 0, 0, Some(e.to_string()), None),
                };
                em.emit(RunEventBody::AgentFinished {
                    agent_id: id.clone(),
                    outcome: outcome.into(),
                    tokens_in: ti,
                    tokens_out: to,
                    error: err,
                    summary,
                });
                (id, res)
            }
        },
        move |done, total| {
            em_prog.emit(RunEventBody::RunProgress {
                agents_done: done,
                agents_total: total,
                agents_running: 0,
                agents_queued: total.saturating_sub(done),
            });
        },
    )
    .await;

    // fold results back into run.steps by StepId
    for (id, res) in results {
        if let Some(st) = run.steps.iter_mut().find(|s| s.spec.id == id) {
            match res {
                Ok(o) => {
                    st.status = StepStatus::Ok;
                    st.tokens_in = o.tokens_in;
                    st.tokens_out = o.tokens_out;
                    st.output = Some(o.output);
                }
                Err(e) => {
                    st.status = StepStatus::Failed;
                    st.error = Some(e.to_string());
                }
            }
        }
    }
    // steps that never reported (panicked, or skipped because cancel fired at entry)
    let cancelled = cancel.is_cancelled();
    for st in run.steps.iter_mut() {
        if st.status == StepStatus::Queued {
            st.status = if cancelled { StepStatus::Cancelled } else { StepStatus::Skipped };
        }
    }
    // A step that panicked emitted AgentSpawned but never AgentFinished (run_batch's
    // catch_unwind dropped the closure), leaving the UI's lamp stuck on "running". When the
    // run was NOT cancelled, a Skipped step is exactly a panicked one — emit its terminal
    // event so every spawn is balanced.
    if !cancelled {
        for st in run.steps.iter().filter(|s| s.status == StepStatus::Skipped) {
            em.emit(RunEventBody::AgentFinished {
                agent_id: st.spec.id.clone(),
                outcome: "err".into(),
                tokens_in: 0,
                tokens_out: 0,
                error: Some("panicked (isolated)".into()),
                summary: None,
            });
        }
    }
    let ok = run.steps.iter().filter(|s| s.status == StepStatus::Ok).count();
    let err = run.steps.iter().filter(|s| s.status == StepStatus::Failed).count();

    // ── verify (adversarial critique, the ultracode review stage) ──
    em.emit(RunEventBody::PhaseChanged { phase: "verifying".into(), index: 2, total: 5 });
    run.status = RunStatus::Verifying;
    let critique = if cancelled {
        String::new()
    } else {
        let vid = format!("{}-verify", opts.run_id);
        em.emit(RunEventBody::AgentSpawned {
            agent_id: vid.clone(),
            role: "reviewer".into(),
            model: opts.model.clone(),
            vendor: opts.provider.clone(),
            phase: "verifying".into(),
        });
        let c = synthesize::critique(goal, &run.steps, &opts.provider, &opts.model, cancel).await;
        em.emit(RunEventBody::AgentFinished {
            agent_id: vid,
            outcome: "ok".into(),
            tokens_in: 0,
            tokens_out: 0,
            error: None,
            summary: Some(if c.trim().is_empty() { "no critique".into() } else { truncate(&c, 120) }),
        });
        c
    };

    run.review = if critique.trim().is_empty() { None } else { Some(critique.clone()) };
    // ── synthesize (addressing the critique) ──
    em.emit(RunEventBody::PhaseChanged { phase: "synthesizing".into(), index: 3, total: 5 });
    run.status = RunStatus::Synthesizing;
    run.synthesis = Some(synthesize::synthesize(goal, &run.steps, &critique, &opts.provider, &opts.model, cancel).await);

    run.status = if cancelled { RunStatus::Cancelled } else { RunStatus::Done };
    em.emit(RunEventBody::PhaseChanged { phase: "done".into(), index: 4, total: 5 });
    em.emit(RunEventBody::RunFinished {
        status: if cancelled { "cancelled".into() } else { "done".into() },
        agents_total: total,
        agents_ok: ok,
        agents_err: err,
        error: None,
    });
    run
}

/// Two-level orchestration for a LARGE agent queue: the goal decomposes into `top`
/// sub-tasks, and EACH sub-task fans out into ~`sub` sub-agents — so `top × sub` agents
/// run through the bounded pool (a real, visible queue). Sub-agent outputs roll up per
/// sub-task, then a final verify + synthesize. This is the "assemble many agents" path.
/// Production wiring: LLM decomposition at both levels + the real provider executor.
pub async fn orchestrate_deep(
    goal: &str,
    opts: &OrchestrateOptions,
    top: usize,
    sub: usize,
    sink: Arc<dyn EventSink>,
    cancel: &Cancel,
) -> Run {
    let top_dec = LlmDecomposer { provider: opts.provider.clone(), model: opts.model.clone(), target: Some(top) };
    let sub_dec = Arc::new(LlmDecomposer { provider: opts.provider.clone(), model: opts.model.clone(), target: Some(sub) });
    deep_core(goal, opts, top, sub, &top_dec, sub_dec, Arc::new(ProviderStep), sink, cancel).await
}

/// Live per-agent tally for the deep path, shared across the nested fan-out closures.
#[derive(Default)]
struct DeepTally {
    spawned: AtomicUsize,
    ok: AtomicUsize,
    err: AtomicUsize,
}

/// Generic two-level scheduler behind `orchestrate_deep`, parameterized over the decomposer
/// (both levels) and the agent executor so the whole deep control flow — nested fan-out,
/// per-group roll-up, real agent accounting, cancel, progress — is testable offline with
/// mocks. `sub_dec`/`step_exec` are `Arc` so they can be cloned into each nested closure.
#[allow(clippy::too_many_arguments)]
async fn deep_core<D1, D2, S>(
    goal: &str,
    opts: &OrchestrateOptions,
    top: usize,
    sub: usize,
    top_dec: &D1,
    sub_dec: Arc<D2>,
    step_exec: Arc<S>,
    sink: Arc<dyn EventSink>,
    cancel: &Cancel,
) -> Run
where
    D1: Decomposer,
    D2: Decomposer + 'static,
    S: AgentStep + 'static,
{
    let em = Arc::new(SeqEmitter::new(sink, opts.run_id.clone()));
    let mut run = Run {
        id: opts.run_id.clone(),
        kind: Kind::Workflow,
        goal: goal.to_string(),
        status: RunStatus::Planning,
        steps: Vec::new(),
        synthesis: None,
        review: None,
        created_ms: now_ms(),
    };
    em.emit(RunEventBody::RunStarted { task_kind: "workflow".into(), title: title_of(goal), cancellable: true });

    let top_plan = match top_dec.plan(goal, &opts.run_id, cancel).await {
        Ok(p) => p,
        Err(e) => return fail(run, &em, e.to_string()),
    };
    let est_total = top_plan.steps.len().max(1) * sub;
    em.emit(RunEventBody::PlanReady {
        phases: vec!["planning".into(), "executing".into(), "verifying".into(), "synthesizing".into(), "done".into()],
        planned_agents: est_total,
        notes: vec![format!("deep · {} 组 × ~{} = {} agents", top_plan.steps.len(), sub, est_total)],
    });
    if cancel.is_cancelled() {
        run.status = RunStatus::Cancelled;
        em.emit(RunEventBody::RunFinished { status: "cancelled".into(), agents_total: est_total, agents_ok: 0, agents_err: 0, error: None });
        return run;
    }
    run.status = RunStatus::Running;
    em.emit(RunEventBody::PhaseChanged { phase: "executing".into(), index: 1, total: 5 });

    // Honor opts.concurrency across the two levels: split the budget so outer × inner ≈
    // concurrency (previously both were hardcoded to 3, ignoring the requested parallelism).
    let cc = opts.concurrency.max(1);
    let outer_cap = ((cc as f64).sqrt().ceil() as usize).clamp(1, top.max(1));
    let inner_cap = cc.div_ceil(outer_cap).max(1);

    let provider = opts.provider.clone();
    let model = opts.model.clone();
    let tally = Arc::new(DeepTally::default());
    let live: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let em_task = em.clone();
    let tally_task = tally.clone();
    let live_task = live.clone();
    // each top-level sub-task runs its OWN nested fan-out of sub-agents
    let top_results = run_batch(
        top_plan.steps.clone(),
        outer_cap,
        cancel,
        move |subtask| {
            let em = em_task.clone();
            let provider = provider.clone();
            let model = model.clone();
            let sub_dec = sub_dec.clone();
            let step_exec = step_exec.clone();
            let tally = tally_task.clone();
            let live = live_task.clone();
            async move {
                let subplan = sub_dec.plan(&subtask.goal, &subtask.id, cancel).await.unwrap_or(Plan { steps: Vec::new() });
                let mut states: Vec<StepState> = subplan.steps.iter().cloned().map(StepState::new).collect();
                let em_sub = em.clone();
                let subrole = subtask.role.clone();
                let sub_results = run_batch(
                    subplan.steps.clone(),
                    inner_cap,
                    cancel,
                    move |spec| {
                        let em = em_sub.clone();
                        let step_exec = step_exec.clone();
                        let tally = tally.clone();
                        let live = live.clone();
                        let role = format!("{subrole} · {}", spec.role);
                        async move {
                            let id = spec.id.clone();
                            tally.spawned.fetch_add(1, Ordering::Relaxed);
                            live.lock().unwrap().insert(id.clone());
                            em.emit(RunEventBody::AgentSpawned {
                                agent_id: id.clone(),
                                role,
                                model: spec.model.clone(),
                                vendor: spec.provider.clone(),
                                phase: "executing".into(),
                            });
                            let input = StepInput { spec, dep_outputs: Vec::new() };
                            let r = step_exec.run(input, cancel).await;
                            let (outcome, err, summary) = match &r {
                                Ok(o) => {
                                    tally.ok.fetch_add(1, Ordering::Relaxed);
                                    ("ok", None, Some(truncate(&o.output, 120)))
                                }
                                Err(e) => {
                                    tally.err.fetch_add(1, Ordering::Relaxed);
                                    ("err", Some(e.to_string()), None)
                                }
                            };
                            live.lock().unwrap().remove(&id);
                            em.emit(RunEventBody::AgentFinished { agent_id: id.clone(), outcome: outcome.into(), tokens_in: 0, tokens_out: 0, error: err, summary });
                            // real per-agent progress (agents_total stays the announced est)
                            let done = tally.ok.load(Ordering::Relaxed) + tally.err.load(Ordering::Relaxed);
                            let spawned = tally.spawned.load(Ordering::Relaxed);
                            let total = est_total.max(spawned);
                            em.emit(RunEventBody::RunProgress {
                                agents_done: done,
                                agents_total: total,
                                agents_running: spawned.saturating_sub(done),
                                agents_queued: total.saturating_sub(spawned),
                            });
                            (id, r)
                        }
                    },
                    |_, _| {},
                )
                .await;
                for (id, r) in sub_results {
                    if let Some(st) = states.iter_mut().find(|s| s.spec.id == id) {
                        match r {
                            Ok(o) => { st.status = StepStatus::Ok; st.output = Some(o.output); }
                            Err(e) => { st.status = StepStatus::Failed; st.error = Some(e.to_string()); }
                        }
                    }
                }
                // a group with sub-agents but zero successes is a FAILED group (was always Ok)
                let group_ok = states.iter().filter(|s| s.status == StepStatus::Ok).count();
                let group_failed = !states.is_empty() && group_ok == 0;
                let sub_synth = synthesize::synthesize(&subtask.goal, &states, "", &provider, &model, cancel).await;
                (subtask.id.clone(), subtask.role.clone(), sub_synth, group_failed)
            }
        },
        // Outer progress is per-GROUP, not per-agent; a no-op here avoids clobbering the
        // announced agent total. Accurate progress is emitted from the inner finishes above.
        |_, _| {},
    )
    .await;

    // Any sub-agent still "live" here panicked (catch_unwind dropped its finish): balance
    // the event stream and count it as an error so no lamp stays stuck on "running".
    let leftover: Vec<String> = { live.lock().unwrap().drain().collect() };
    for id in leftover {
        tally.err.fetch_add(1, Ordering::Relaxed);
        em.emit(RunEventBody::AgentFinished {
            agent_id: id,
            outcome: "err".into(),
            tokens_in: 0,
            tokens_out: 0,
            error: Some("panicked (isolated)".into()),
            summary: None,
        });
    }

    run.steps = top_results
        .iter()
        .map(|(id, role, synth, group_failed)| {
            let mut st = StepState::new(StepSpec { id: id.clone(), role: role.clone(), goal: String::new(), provider: opts.provider.clone(), model: opts.model.clone(), deps: Vec::new(), max_tokens: None });
            st.status = if *group_failed { StepStatus::Failed } else { StepStatus::Ok };
            st.output = Some(synth.clone());
            st
        })
        .collect();

    let cancelled = cancel.is_cancelled();
    em.emit(RunEventBody::PhaseChanged { phase: "verifying".into(), index: 2, total: 5 });
    run.status = RunStatus::Verifying;
    let critique = if cancelled {
        String::new()
    } else {
        let vid = format!("{}-verify", opts.run_id);
        em.emit(RunEventBody::AgentSpawned { agent_id: vid.clone(), role: "reviewer".into(), model: opts.model.clone(), vendor: opts.provider.clone(), phase: "verifying".into() });
        let c = synthesize::critique(goal, &run.steps, &opts.provider, &opts.model, cancel).await;
        em.emit(RunEventBody::AgentFinished { agent_id: vid, outcome: "ok".into(), tokens_in: 0, tokens_out: 0, error: None, summary: Some(if c.trim().is_empty() { "no critique".into() } else { truncate(&c, 120) }) });
        c
    };
    run.review = if critique.trim().is_empty() { None } else { Some(critique.clone()) };
    em.emit(RunEventBody::PhaseChanged { phase: "synthesizing".into(), index: 3, total: 5 });
    run.status = RunStatus::Synthesizing;
    run.synthesis = Some(synthesize::synthesize(goal, &run.steps, &critique, &opts.provider, &opts.model, cancel).await);
    run.status = if cancelled { RunStatus::Cancelled } else { RunStatus::Done };
    em.emit(RunEventBody::PhaseChanged { phase: "done".into(), index: 4, total: 5 });
    // Real sub-agent accounting (was: agents_ok = group count, agents_err hardcoded 0).
    let ok = tally.ok.load(Ordering::Relaxed);
    let err = tally.err.load(Ordering::Relaxed);
    em.emit(RunEventBody::RunFinished { status: if cancelled { "cancelled".into() } else { "done".into() }, agents_total: ok + err, agents_ok: ok, agents_err: err, error: None });
    run
}

fn fail(mut run: Run, em: &SeqEmitter, msg: String) -> Run {
    run.status = RunStatus::Failed;
    em.emit(RunEventBody::RunFinished { status: "failed".into(), agents_total: 0, agents_ok: 0, agents_err: 0, error: Some(msg) });
    run
}

fn title_of(goal: &str) -> String {
    truncate(goal.trim(), 60)
}

fn truncate(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use events::{RunEventBody as B, VecSink};
    use step::MockStep;
    use supervisor::MockDecomposer;

    fn opts() -> OrchestrateOptions {
        OrchestrateOptions { run_id: "run-test".into(), provider: "mock".into(), model: "mock".into(), concurrency: 2 }
    }
    fn dec(pairs: &[(&str, &str)]) -> MockDecomposer {
        MockDecomposer { specs: pairs.iter().map(|(r, g)| (r.to_string(), g.to_string())).collect() }
    }

    #[tokio::test]
    async fn runs_all_steps_and_streams_ordered_gapfree_events() {
        let sink = Arc::new(VecSink::default());
        let run = orchestrate("goal", &opts(), &dec(&[("a", "do a"), ("b", "do b"), ("c", "do c")]), &MockStep, sink.clone(), &Cancel::new()).await;
        assert_eq!(run.status, RunStatus::Done);
        assert_eq!(run.steps.len(), 3);
        assert!(run.steps.iter().all(|s| s.status == StepStatus::Ok));
        assert!(run.synthesis.is_some());

        let evs = sink.events();
        for (i, e) in evs.iter().enumerate() {
            assert_eq!(e.seq, i as u64, "seq must be gap-free and ordered");
        }
        assert!(matches!(evs.first().unwrap().body, B::RunStarted { .. }));
        assert!(matches!(evs.last().unwrap().body, B::RunFinished { .. }));
        // 3 step agents + 1 reviewer (the verify stage)
        assert_eq!(evs.iter().filter(|e| matches!(e.body, B::AgentSpawned { .. })).count(), 4);
        assert_eq!(evs.iter().filter(|e| matches!(e.body, B::AgentFinished { .. })).count(), 4);
    }

    #[tokio::test]
    async fn per_step_outputs_correlate_to_their_id() {
        let sink = Arc::new(VecSink::default());
        let run = orchestrate("g", &opts(), &dec(&[("a", "alpha"), ("b", "beta")]), &MockStep, sink, &Cancel::new()).await;
        let a = run.steps.iter().find(|s| s.spec.id == "run-test-s0").unwrap();
        assert_eq!(a.output.as_deref(), Some("done: alpha"));
        let b = run.steps.iter().find(|s| s.spec.id == "run-test-s1").unwrap();
        assert_eq!(b.output.as_deref(), Some("done: beta"));
    }

    #[tokio::test]
    async fn concurrency_cap_is_respected() {
        // 5 steps at concurrency 2 still all complete (run_batch bounds in-flight).
        let sink = Arc::new(VecSink::default());
        let run = orchestrate("g", &opts(), &dec(&[("a", "1"), ("b", "2"), ("c", "3"), ("d", "4"), ("e", "5")]), &MockStep, sink, &Cancel::new()).await;
        assert_eq!(run.steps.iter().filter(|s| s.status == StepStatus::Ok).count(), 5);
    }

    #[tokio::test]
    async fn panicking_step_is_isolated_and_run_still_completes() {
        let sink = Arc::new(VecSink::default());
        let run = orchestrate("g", &opts(), &dec(&[("a", "ok one"), ("b", "PANIC"), ("c", "ok two")]), &MockStep, sink.clone(), &Cancel::new()).await;
        assert_eq!(run.status, RunStatus::Done);
        let panicked = run.steps.iter().find(|s| s.spec.id == "run-test-s1").unwrap();
        assert_eq!(panicked.status, StepStatus::Skipped, "panicked step has no result → skipped");
        assert_eq!(run.steps.iter().filter(|s| s.status == StepStatus::Ok).count(), 2);
        assert!(run.synthesis.is_some());
        let evs = sink.events();
        // 3 step spawns + 1 reviewer = 4; the panicked step now gets a SYNTHETIC terminal
        // event (2 ok steps + 1 panicked + reviewer = 4) so no spawn is left unbalanced.
        assert_eq!(evs.iter().filter(|e| matches!(e.body, B::AgentSpawned { .. })).count(), 4);
        assert_eq!(evs.iter().filter(|e| matches!(e.body, B::AgentFinished { .. })).count(), 4);
        // every spawned agent has a matching finish (no stuck "running" lamp)
        let spawned: HashSet<_> = evs.iter().filter_map(|e| match &e.body {
            B::AgentSpawned { agent_id, .. } => Some(agent_id.clone()),
            _ => None,
        }).collect();
        let finished: HashSet<_> = evs.iter().filter_map(|e| match &e.body {
            B::AgentFinished { agent_id, .. } => Some(agent_id.clone()),
            _ => None,
        }).collect();
        assert_eq!(spawned, finished, "every AgentSpawned must have an AgentFinished");
    }

    #[tokio::test]
    async fn failing_step_marks_failed_but_run_done() {
        let sink = Arc::new(VecSink::default());
        let run = orchestrate("g", &opts(), &dec(&[("a", "ok"), ("b", "FAIL")]), &MockStep, sink, &Cancel::new()).await;
        assert_eq!(run.status, RunStatus::Done);
        assert_eq!(run.steps.iter().filter(|s| s.status == StepStatus::Failed).count(), 1);
        assert_eq!(run.steps.iter().filter(|s| s.status == StepStatus::Ok).count(), 1);
    }

    #[tokio::test]
    async fn precancelled_run_does_no_agent_work() {
        let sink = Arc::new(VecSink::default());
        let cancel = Cancel::new();
        cancel.cancel();
        let run = orchestrate("g", &opts(), &dec(&[("a", "do a"), ("b", "do b")]), &MockStep, sink.clone(), &cancel).await;
        assert_eq!(run.status, RunStatus::Cancelled);
        assert_eq!(sink.events().iter().filter(|e| matches!(e.body, B::AgentFinished { .. })).count(), 0);
    }

    // ── deep (two-level) orchestration, now testable offline via deep_core ──

    fn run_finished(evs: &[events::RunEvent]) -> (usize, usize, usize) {
        evs.iter()
            .rev()
            .find_map(|e| match &e.body {
                B::RunFinished { agents_total, agents_ok, agents_err, .. } => {
                    Some((*agents_total, *agents_ok, *agents_err))
                }
                _ => None,
            })
            .unwrap()
    }

    #[tokio::test]
    async fn deep_counts_real_subagents_not_groups() {
        let sink = Arc::new(VecSink::default());
        let top = dec(&[("g1", "group one"), ("g2", "group two")]); // 2 groups
        let sub = Arc::new(dec(&[("a", "sa"), ("b", "sb"), ("c", "sc")])); // 3 sub-agents / group
        let run = deep_core("goal", &opts(), 2, 3, &top, sub, Arc::new(MockStep), sink.clone(), &Cancel::new()).await;
        assert_eq!(run.status, RunStatus::Done);
        assert_eq!(run.steps.len(), 2, "2 groups → 2 rolled-up steps");
        assert!(run.steps.iter().all(|s| s.status == StepStatus::Ok));
        // gap-free ordering holds through the nested fan-out
        let evs = sink.events();
        for (i, e) in evs.iter().enumerate() {
            assert_eq!(e.seq, i as u64, "seq must stay gap-free under nested concurrency");
        }
        // RunFinished reports the 6 REAL sub-agents (2×3), not the 2 groups
        assert_eq!(run_finished(&evs), (6, 6, 0));
        assert_eq!(evs.iter().filter(|e| matches!(e.body, B::AgentSpawned { .. })).count(), 7); // 6 + reviewer
    }

    #[tokio::test]
    async fn deep_marks_all_failed_group_as_failed_and_counts_errors() {
        let sink = Arc::new(VecSink::default());
        let top = dec(&[("g1", "group one")]);
        let sub = Arc::new(dec(&[("a", "FAIL"), ("b", "FAIL")])); // every sub-agent errors
        let run = deep_core("g", &opts(), 1, 2, &top, sub, Arc::new(MockStep), sink.clone(), &Cancel::new()).await;
        assert_eq!(run.steps.len(), 1);
        assert_eq!(run.steps[0].status, StepStatus::Failed, "a group with zero successes is Failed, not Ok");
        // agents_err is real (was hardcoded 0)
        assert_eq!(run_finished(&sink.events()), (2, 0, 2));
    }

    #[tokio::test]
    async fn deep_balances_events_on_subagent_panic() {
        let sink = Arc::new(VecSink::default());
        let top = dec(&[("g1", "group one")]);
        let sub = Arc::new(dec(&[("a", "ok one"), ("b", "PANIC")]));
        let run = deep_core("g", &opts(), 1, 2, &top, sub, Arc::new(MockStep), sink.clone(), &Cancel::new()).await;
        assert_eq!(run.status, RunStatus::Done);
        let evs = sink.events();
        let spawned: HashSet<_> = evs.iter().filter_map(|e| match &e.body {
            B::AgentSpawned { agent_id, .. } => Some(agent_id.clone()),
            _ => None,
        }).collect();
        let finished: HashSet<_> = evs.iter().filter_map(|e| match &e.body {
            B::AgentFinished { agent_id, .. } => Some(agent_id.clone()),
            _ => None,
        }).collect();
        assert_eq!(spawned, finished, "a panicked sub-agent still emits a terminal event");
    }
}
