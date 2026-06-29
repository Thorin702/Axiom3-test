// Bilingual layer. The backend speaks language-invariant machine codes
// (verdict status, checker codes, reason codes); the UI maps them to labels here,
// so adding a language never touches Rust.

export type Lang = "zh" | "en";

type Pair = { zh: string; en: string };

const UI: Record<string, Pair> = {
  app_title: { zh: "Axiom3 · 工作台", en: "Axiom3 · workbench" },
  app_subtitle: {
    zh: "每个项目一个加密的系统记录(SQLCipher + 哈希链事件日志),叠加跨厂商校验。",
    en: "A per-project encrypted system-of-record (SQLCipher + hash-chained event log), with cross-vendor verification.",
  },
  project_placeholder: { zh: "项目名…", en: "project name…" },
  open_create: { zh: "打开 / 新建", en: "Open / create" },
  close: { zh: "关闭", en: "Close" },
  chain_intact: { zh: "链完整 ✓", en: "chain intact ✓" },
  chain_broken: { zh: "链已损坏 ✗", en: "chain BROKEN ✗" },
  events: { zh: "事件", en: "events" },
  head: { zh: "链头", en: "head" },
  actor: { zh: "操作者", en: "actor" },
  op: { zh: "操作", en: "op" },
  payload: { zh: "载荷 (JSON)", en: "payload (JSON)" },
  append_event: { zh: "追加事件", en: "Append event" },
  verify_chain: { zh: "验证链", en: "Verify chain" },
  vector_self_test: { zh: "向量自检", en: "Vector self-test" },
  check_title: { zh: "校对草稿(确定性 + 跨厂商)", en: "Check a draft (deterministic + cross-vendor)" },
  check_placeholder: {
    zh: "粘贴一段草稿…每句话会变成一条断言并接受校验。",
    en: "Paste a draft… each sentence becomes a claim and is verified.",
  },
  check_btn: { zh: "校对草稿", en: "Check draft" },
  claims: { zh: "条断言", en: "claims" },
  flagged: { zh: "条已标记", en: "flagged" },
  cross_vendor_n: { zh: "条跨厂商核验", en: "cross-vendor" },
  event_no: { zh: "事件号", en: "event" },
  activity: { zh: "活动", en: "activity" },
  cross_vendor: { zh: "跨厂商", en: "cross-vendor" },
  opened_project: { zh: "已打开项目", en: "opened project" },
  closed_project: { zh: "已关闭项目", en: "closed project" },
  checked_draft: { zh: "已校对草稿", en: "checked draft" },
};

export function t(lang: Lang, key: string): string {
  return UI[key]?.[lang] ?? key;
}

const VERDICT: Record<string, Pair> = {
  verified: { zh: "已核实", en: "verified" },
  refuted: { zh: "被驳斥", en: "refuted" },
  uncertain: { zh: "存疑", en: "uncertain" },
  quarantined: { zh: "已隔离", en: "quarantined" },
  injection_neutralized: { zh: "注入已中和", en: "injection neutralized" },
};

export function verdictLabel(lang: Lang, code: string): string {
  return VERDICT[code]?.[lang] ?? code;
}

const INDEP: Record<string, Pair> = {
  multi_family: { zh: "多厂商", en: "multi-family" },
  single_family: { zh: "单厂商", en: "single-family" },
  local_only: { zh: "仅本地", en: "local-only" },
};

export function indepLabel(lang: Lang, code: string): string {
  return INDEP[code]?.[lang] ?? code;
}

// Reason + checker tokens (a reason string is comma-joined codes).
const CODE: Record<string, Pair> = {
  overclaim: { zh: "夸大宣称", en: "over-claim" },
  absolute_quantifier: { zh: "绝对量词", en: "absolute quantifier" },
  uncited_quantitative: { zh: "数值无引用", en: "uncited quantitative" },
  internal_conflict: { zh: "内部数值冲突", en: "internal numeric conflict" },
  multi_family_supported: { zh: "多厂商一致支持", en: "multi-family supported" },
  multi_family_refuted: { zh: "多厂商一致驳斥", en: "multi-family refuted" },
  cross_vendor_disagreement: { zh: "跨厂商分歧", en: "cross-vendor disagreement" },
  single_family_independence_unmet: { zh: "独立性不足(单厂商)", en: "single-family (independence unmet)" },
  insufficient_corroboration: { zh: "佐证不足", en: "insufficient corroboration" },
  deterministic_flags_block_verify: { zh: "确定性标记阻止核实", en: "deterministic flags block verify" },
  clean_pending: { zh: "无确定性问题(待外部核验)", en: "no deterministic issue (pending)" },
};

export function codeLabel(lang: Lang, code: string): string {
  return CODE[code]?.[lang] ?? code;
}

export function reasonLabel(lang: Lang, reason: string | null): string {
  if (!reason) return "";
  const sep = lang === "zh" ? "、" : ", ";
  return reason
    .split(",")
    .map((c) => codeLabel(lang, c.trim()))
    .join(sep);
}

// A localized one-line explanation for each deterministic checker finding.
const FINDING_DESC: Record<string, Pair> = {
  overclaim: { zh: "新颖性/最高级宣称,需外部佐证", en: "novelty/superiority claim — needs external support" },
  absolute_quantifier: { zh: "绝对量词,极少字面成立", en: "absolute quantifier — rarely literally true" },
  uncited_quantitative: { zh: "有数值却无引用", en: "a quantitative claim with no citation" },
  internal_conflict: { zh: "与他处数值冲突,请确认条件是否不同", en: "differs from a value stated elsewhere — confirm the conditions differ" },
};

export function findingDesc(lang: Lang, checker: string): string {
  return FINDING_DESC[checker]?.[lang] ?? checker;
}
