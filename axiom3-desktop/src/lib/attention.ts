// Map a claim's machine status + findings onto an ATTENTION state — never a truth
// verdict. The red line: Axiom3 checks and surfaces evidence; it does not adjudicate
// right/wrong. So a green lamp means "checks ran, nothing to flag" — NOT "true"; an
// amber lamp means "look at this", not "false".

import type { ClaimEntry } from "./types";

export type Attention = "flagged" | "review" | "clear" | "neutralized";

export function attentionOf(e: ClaimEntry): Attention {
  // Untrusted reference text was neutralized — its own terminal state.
  if (e.status === "injection_neutralized" || e.status === "quarantined") return "neutralized";
  // A deterministic check fired, or a source contradicts → something to look at.
  if (e.findings.length > 0) return "flagged";
  if (e.status === "refuted") return "flagged";
  // The cross-model read supported it with no flags → checks ran, nothing flagged.
  if (e.status === "verified") return "clear";
  // Otherwise uncertain / pending: a bearing conclusion still wants your eyes; a plain
  // descriptive sentence with nothing flagged is clear.
  return e.is_bearing ? "review" : "clear";
}

export const lampClass: Record<Attention, string> = {
  flagged: "is-amber",
  review: "is-neutral",
  clear: "is-green",
  neutralized: "is-violet",
};

export const attentionKey: Record<Attention, string> = {
  flagged: "att_flagged",
  review: "att_review",
  clear: "att_clear",
  neutralized: "att_neutralized",
};
