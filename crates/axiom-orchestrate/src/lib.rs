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
pub mod model;
pub mod step;
pub mod supervisor;
pub mod synthesize;

pub use control::Pause;

use events::{now_ms, EventSink, RunEventBody, SeqEmitter};
use model::{Kind, Plan, Run, RunStatus, StepState, StepStatus};
use std::sync::Arc;
use step::{AgentStep, StepInput};
use supervisor::Decomposer;

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
        // it was spawned (before the panic) but never finished
        let evs = sink.events();
        // 3 step spawns + 1 reviewer = 4; finishes = 2 ok steps + reviewer = 3 (panicked one never finishes)
        assert_eq!(evs.iter().filter(|e| matches!(e.body, B::AgentSpawned { .. })).count(), 4);
        assert_eq!(evs.iter().filter(|e| matches!(e.body, B::AgentFinished { .. })).count(), 3);
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
}
