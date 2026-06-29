//! Axiom3 model-access layer.
//!
//! Provider-agnostic plumbing that sits between the agent runtime and the model
//! APIs: a canonical message model ([`message`]), the per-model capability matrix
//! ([`capability`]), and role-based routing with the cross-vendor independence gate
//! ([`routing`]). Concrete provider adapters (and the Rig `cache_control` /
//! thinking-replay fidelity spike) land on top of these types.

pub mod capability;
pub mod message;
pub mod routing;

pub use capability::{CapabilityMatrix, Modality, ModelCapability, ModelStatus};
pub use message::{ContentBlock, Message, Request, Role, ToolDef};
pub use routing::{bearing_independence_met, independence, route, AgentRole, RouteDecision, RouteRequest};
