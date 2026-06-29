//! Canonical request/message model.
//!
//! Modelled on the RICHER Anthropic block shape (typed content blocks, a separate
//! `system` field, signed thinking blocks, cache hints) and down-converted to
//! OpenAI by each adapter — choosing the OpenAI Chat-Completions shape as canonical
//! would force lossy hacks (SSOT 设计对齐 §4).

use serde::{Deserialize, Serialize};

/// Conversation roles. `system` is a separate top-level field on [`Request`]
/// (Anthropic-style), not a role here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// One typed content block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Plain text. `cache` requests an Anthropic `cache_control` breakpoint at this
    /// block (no-op for OpenAI's automatic caching).
    Text {
        text: String,
        #[serde(default)]
        cache: bool,
    },
    /// Extended-thinking block. `signature` is an OPAQUE, model-bound payload:
    /// store and replay it UNCHANGED within a same-model multi-turn loop; never
    /// strip, mutate, or move it across model families (SSOT 设计对齐 §4 / §11).
    Thinking {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    /// A tool/function call requested by the model.
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// The result of executing a tool call.
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
    /// An image input. `data_ref` is a CAS BLAKE3 hash or URL, not inline bytes,
    /// so messages stay small and evidence stays content-addressed.
    Image {
        media_type: String,
        data_ref: String,
    },
}

impl ContentBlock {
    /// Convenience constructor for an uncached text block.
    pub fn text(s: impl Into<String>) -> Self {
        ContentBlock::Text { text: s.into(), cache: false }
    }
}

/// One message in the conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    pub fn user(blocks: Vec<ContentBlock>) -> Self {
        Self { role: Role::User, content: blocks }
    }
    pub fn assistant(blocks: Vec<ContentBlock>) -> Self {
        Self { role: Role::Assistant, content: blocks }
    }
}

/// A tool/function the model may call (canonical shape; adapters map to each wire format).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool input.
    pub input_schema: serde_json::Value,
}

/// A canonical completion request. Adapters translate this to each provider's wire
/// format; the round-trip fidelity of `cache`/`Thinking.signature` is exactly the
/// P0 risk spike.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Top-level system prompt (Anthropic places it here; OpenAI adapter folds it
    /// into a leading system message).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Per-provider reasoning knob (`reasoning_effort` for OpenAI, thinking effort
    /// for Anthropic) — never a raw deprecated `budget_tokens`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDef>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thinking_signature_survives_serde_roundtrip() {
        let block = ContentBlock::Thinking {
            text: "step 1...".into(),
            signature: Some("opaque-sig-abc".into()),
        };
        let json = serde_json::to_string(&block).unwrap();
        let back: ContentBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(block, back, "signed thinking block must round-trip unchanged");
    }

    #[test]
    fn text_cache_hint_defaults_false_and_is_tagged() {
        let json = serde_json::to_string(&ContentBlock::text("hi")).unwrap();
        assert!(json.contains("\"type\":\"text\""));
        // cache defaults to false; an explicit cache=true is preserved.
        let cached = ContentBlock::Text { text: "sys".into(), cache: true };
        let back: ContentBlock = serde_json::from_str(&serde_json::to_string(&cached).unwrap()).unwrap();
        assert_eq!(cached, back);
    }
}
