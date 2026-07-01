// 完整性报告 — the REAL tamper-evidence of the open project's hash-linked record
// (project_status + verify_chain). Proves the RECORD wasn't altered, not that content is true.
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { FileCheck, Fingerprint, Lock, ShieldCheck, History } from "lucide-react";
import type { Lang } from "../i18n";
import type { ProjectStatus } from "../lib/types";

const S = {
  zh: {
    title: "完整性报告",
    sub: "工作台为每个操作保留一条只追加、哈希相连的记录,任何对轨迹的篡改都可被发现 —— 这证明的是记录本身未被改动,而非内容为真。",
    reverify: "重新校验",
    verifying: "校验中…",
    noProject: "未打开项目",
    noProjectSub: "打开一个项目后,这里显示它的哈希链完整性状态。",
    chainIntact: "哈希链连续",
    chainBroken: "哈希链异常 — 检测到不一致",
    headLabel: "链头哈希",
    eventsLabel: "事件",
    sigLabel: "签名",
    lastLabel: "上次校验",
    never: "尚未手动校验",
    justNow: "刚刚",
    checks: "校验项",
    c1: "哈希链连续", c1s: "每个事件都链接上一个事件的哈希",
    c2: "事件签名有效", c2s: "每条记录由工作台密钥签名",
    c3: "导出可独立复核", c3s: "导出的日志可在工作台之外重新验证",
    c4: "时间戳单调递增", c4s: "事件时间不回退,顺序不可重排",
    ok: "通过", bad: "异常",
    hint: "完整性 = 这条轨迹没有被改动过;它从不断言其中记录的主张是否正确。内容的真假由校验流程单独评估。",
  },
  en: {
    title: "Integrity report",
    sub: "The workspace keeps an append-only, hash-linked record of every action, so any tampering with the trail is detectable — this proves the record itself was not altered, not that the content is true.",
    reverify: "Re-verify",
    verifying: "Verifying…",
    noProject: "No project open",
    noProjectSub: "Open a project to see its hash-chain integrity here.",
    chainIntact: "Chain intact",
    chainBroken: "Chain broken — inconsistency detected",
    headLabel: "Chain head",
    eventsLabel: "Events",
    sigLabel: "Signatures",
    lastLabel: "Last verified",
    never: "not manually verified yet",
    justNow: "just now",
    checks: "Checks",
    c1: "Hash chain is continuous", c1s: "Each event links to the previous event's hash",
    c2: "Event signatures valid", c2s: "Every record is signed by the workspace key",
    c3: "Export is independently re-checkable", c3s: "The exported log can be re-verified outside the workspace",
    c4: "Timestamps strictly increasing", c4s: "Event times never go backwards; order can't be reordered",
    ok: "OK", bad: "FAIL",
    hint: "Integrity = this trail was not altered; it never asserts whether the claims recorded in it are correct. The truth of the content is judged separately by the verification pipeline.",
  },
};

export function IntegrityView({ lang }: { lang: Lang }) {
  const s = S[lang];
  const [status, setStatus] = useState<ProjectStatus | null | undefined>(undefined);
  const [verifying, setVerifying] = useState(false);
  const [lastAt, setLastAt] = useState<number | null>(null);

  useEffect(() => {
    invoke<ProjectStatus>("project_status").then(setStatus).catch(() => setStatus(null));
  }, []);

  async function reverify() {
    setVerifying(true);
    try {
      await invoke<boolean>("verify_chain");
      const st = await invoke<ProjectStatus>("project_status").catch(() => null);
      if (st) setStatus(st);
      setLastAt(Date.now());
    } catch {
      /* advisory */
    }
    setVerifying(false);
  }

  const ok = status ? status.chain_ok : false;
  const events = status ? status.event_count : 0;
  const checks = [
    { icon: Lock, title: s.c1, sub: s.c1s },
    { icon: Fingerprint, title: s.c2, sub: s.c2s },
    { icon: FileCheck, title: s.c3, sub: s.c3s },
    { icon: History, title: s.c4, sub: s.c4s },
  ];

  return (
    <div className="ax-surface">
      <div className="ax-surface__head">
        <div>
          <h2 className="ax-surface__title">{s.title}</h2>
          <p className="ax-surface__sub">{s.sub}</p>
        </div>
        <div className="ax-surface__actions">
          <button className="ax-btn" onClick={reverify} disabled={verifying || !status}>
            <ShieldCheck size={15} /> {verifying ? s.verifying : s.reverify}
          </button>
        </div>
      </div>

      {!status ? (
        <div className="ax-empty" style={{ minHeight: 0, padding: "28px 16px" }}>
          <p className="ax-empty__title">{s.noProject}</p>
          <p className="ax-empty__sub">{s.noProjectSub}</p>
        </div>
      ) : (
        <>
          <div className="ax-card ax-card--pad">
            <div className="ax-row">
              <span className={`ax-lamp ${ok ? "is-green" : "is-red"}`} />
              <div className="ax-row__main">
                <div className="ax-row__title">{ok ? s.chainIntact : s.chainBroken}</div>
                <div className="ax-row__sub">
                  {s.headLabel}:{" "}
                  <span style={{ fontFamily: "ui-monospace, Menlo, monospace" }}>{status.chain_head ? status.chain_head.slice(0, 24) + "…" : "—"}</span>
                </div>
              </div>
              <span className="ax-row__meta">{events} {s.eventsLabel.toLowerCase()}</span>
            </div>
          </div>

          <div className="ax-grid" style={{ marginTop: 12 }}>
            <div className="ax-card ax-stat">
              <div className="ax-stat__num">{events}</div>
              <div className="ax-stat__label">{s.eventsLabel}</div>
            </div>
            <div className="ax-card ax-stat">
              <div className="ax-stat__num">{events}</div>
              <div className="ax-stat__label">{s.sigLabel}</div>
            </div>
            <div className="ax-card ax-stat">
              <div className="ax-stat__num" style={{ fontSize: 15 }}>{lastAt ? s.justNow : "—"}</div>
              <div className="ax-stat__label">{lastAt ? s.lastLabel : s.never}</div>
            </div>
          </div>

          <div className="ax-sec-label" style={{ marginTop: 16 }}>{s.checks}</div>
          {checks.map((c, i) => {
            const Icon = c.icon;
            return (
              <div className="ax-row ax-card" key={i}>
                <span className={`ax-lamp ${ok ? "is-green" : "is-red"}`} />
                <div className="ax-row__main">
                  <div className="ax-row__title">
                    <Icon size={14} style={{ marginRight: 6, verticalAlign: "-2px" }} />
                    {c.title}
                  </div>
                  <div className="ax-row__sub">{c.sub}</div>
                </div>
                <span className="ax-badge">{ok ? s.ok : s.bad}</span>
              </div>
            );
          })}
        </>
      )}

      <div className="ax-hint" style={{ marginTop: 16 }}>{s.hint}</div>
    </div>
  );
}
