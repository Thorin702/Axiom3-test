import { invoke } from "@tauri-apps/api/core";

/**
 * The interaction names the backend accepts. This union MIRRORS the closed `Feature` enum in
 * src-tauri/src/telemetry.rs — keep the two in sync.
 *
 * It is a union rather than `string` for a specific reason: the Rust side DROPS names it does not
 * recognise (that is what makes the seam un-smuggleable). A typo would therefore not fail, it would
 * silently report nothing — the metric would just read zero forever and look like "nobody uses this
 * feature". Typing it here turns that into a compile error instead.
 */
export type Feat =
  | "export_pdf"
  | "export_word"
  | "export_html"
  | "continue"
  | "cancel"
  | "pause"
  | "fork"
  | "expand_artifact"
  | "close_guard_confirmed"
  | "new_task"
  | "resume_run";

/**
 * Report a UI interaction. Anonymous and content-free: the name below is the ENTIRE payload — no
 * topic, no file, no report text ever goes through here (the backend has no field to put it in).
 * Fire-and-forget; a telemetry failure must never affect what the user was doing.
 *
 * NOTE: not usable immediately before the process exits — the send is async and would be lost. The
 * close guard's confirmation is therefore recorded on the Rust side instead; see `mark_quit_confirmed`.
 */
export function feat(f: Feat) {
  invoke("telemetry", { event: "feature", props: { feature: f } }).catch(() => {});
}
