//! Neutral task/run data model for the orchestration engine.
//!
//! These are LIFECYCLE types only — `Queued/Running/Ok/Failed/...`, never truth
//! verdicts. The engine produces free-form work; the product's truth-checking red line
//! lives entirely in axiom-core and must NOT reach this crate (no axiom-core dep).
use serde::{Deserialize, Serialize};

pub type RunId = String;
pub type StepId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Workflow,
    Coding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Queued,
    Running,
    Ok,
    Failed,
    Cancelled,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Planning,
    Running,
    Synthesizing,
    Done,
    Failed,
    Cancelled,
}

/// One planned sub-agent: what to do, which model runs it, what it depends on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepSpec {
    pub id: StepId,
    pub role: String,
    pub goal: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub deps: Vec<StepId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

/// Live state of a step as the run progresses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepState {
    pub spec: StepSpec,
    pub status: StepStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub tokens_in: u64,
    pub tokens_out: u64,
}

impl StepState {
    pub fn new(spec: StepSpec) -> Self {
        Self { spec, status: StepStatus::Queued, output: None, error: None, tokens_in: 0, tokens_out: 0 }
    }
}

/// A decomposed plan. May be a flat fan-out (no deps) or, later, a DAG.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub steps: Vec<StepSpec>,
}

impl Plan {
    /// Reject a malformed plan from the (untrusted) LLM decomposer: duplicate ids,
    /// dangling deps, or a dependency cycle. Kahn topological sort.
    pub fn validate_acyclic(&self) -> Result<(), String> {
        use std::collections::{HashMap, HashSet, VecDeque};
        let ids: HashSet<&str> = self.steps.iter().map(|s| s.id.as_str()).collect();
        if ids.len() != self.steps.len() {
            return Err("plan has duplicate step ids".into());
        }
        for s in &self.steps {
            for d in &s.deps {
                if !ids.contains(d.as_str()) {
                    return Err(format!("step {} depends on unknown step {}", s.id, d));
                }
            }
        }
        // in-degree = number of deps; adjacency = dep -> dependents
        let mut indeg: HashMap<&str, usize> = self.steps.iter().map(|s| (s.id.as_str(), s.deps.len())).collect();
        let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
        for s in &self.steps {
            for d in &s.deps {
                dependents.entry(d.as_str()).or_default().push(s.id.as_str());
            }
        }
        let mut q: VecDeque<&str> = indeg.iter().filter(|(_, &v)| v == 0).map(|(&k, _)| k).collect();
        let mut seen = 0usize;
        while let Some(n) = q.pop_front() {
            seen += 1;
            if let Some(children) = dependents.get(n) {
                for &c in children {
                    let e = indeg.get_mut(c).expect("id in indeg");
                    *e -= 1;
                    if *e == 0 {
                        q.push_back(c);
                    }
                }
            }
        }
        if seen != self.steps.len() {
            return Err("plan has a dependency cycle".into());
        }
        Ok(())
    }
}

/// A whole orchestration run: the goal, its steps' live state, and the folded result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub id: RunId,
    pub kind: Kind,
    pub goal: String,
    pub status: RunStatus,
    pub steps: Vec<StepState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synthesis: Option<String>,
    pub created_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str, deps: &[&str]) -> StepSpec {
        StepSpec {
            id: id.into(),
            role: "r".into(),
            goal: "g".into(),
            provider: "mock".into(),
            model: "mock".into(),
            deps: deps.iter().map(|s| s.to_string()).collect(),
            max_tokens: None,
        }
    }

    #[test]
    fn flat_plan_is_acyclic() {
        let p = Plan { steps: vec![spec("a", &[]), spec("b", &[]), spec("c", &[])] };
        assert!(p.validate_acyclic().is_ok());
    }

    #[test]
    fn dag_plan_is_acyclic() {
        let p = Plan { steps: vec![spec("a", &[]), spec("b", &["a"]), spec("c", &["a", "b"])] };
        assert!(p.validate_acyclic().is_ok());
    }

    #[test]
    fn cycle_is_rejected() {
        let p = Plan { steps: vec![spec("a", &["b"]), spec("b", &["a"])] };
        assert!(p.validate_acyclic().is_err());
    }

    #[test]
    fn dangling_dep_is_rejected() {
        let p = Plan { steps: vec![spec("a", &["zzz"])] };
        assert!(p.validate_acyclic().is_err());
    }
}
