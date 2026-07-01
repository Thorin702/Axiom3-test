import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { FolderOpen, Plus, FileCheck, ShieldCheck, BookOpen, Boxes } from "lucide-react";
import type { Lang } from "../i18n";

type ProjectStatus = {
  project_id: string;
  root: string;
  event_count: number;
  chain_head: string | null;
  chain_ok: boolean;
};

const S = {
  zh: {
    title: "项目 / Projects",
    sub: "以校验为中心的科研工作区 · 每个项目自带可核验的事件链",
    newBtn: "＋ 新建",
    openLabel: "打开或新建",
    namePh: "项目名称,如 2026-气候论文",
    openBtn: "打开 / 新建",
    opening: "打开中…",
    current: "当前项目",
    recent: "最近项目",
    lastOpened: "上次打开",
    events: "个事件",
    chainOk: "链已核验",
    chainReview: "链待复核",
    chainIdle: "尚未核验",
    emptyHint: "打开或新建一个项目后,这里会显示其事件链状态。以下为示例项目。",
    templates: "模板",
    tplNote: "示例模板 · 选择后按流程建库(即将开放)",
    tpl1: "论文校对",
    tpl1s: "引用核对 · 图表复算 · 交叉模型复核",
    tpl2: "标书审阅",
    tpl2s: "条款比对 · 一致性检查 · 证据留痕",
    tpl3: "代码库分析",
    tpl3s: "结构梳理 · 依赖核查 · 变更追溯",
    ill1: "2026-气候论文",
    ill2: "国自然-标书V3",
    ill3: "axiom3-代码审计",
  },
  en: {
    title: "项目 / Projects",
    sub: "Verification-first research workspace · each project keeps a checkable event chain",
    newBtn: "＋ New",
    openLabel: "Open or create",
    namePh: "Project name, e.g. 2026-climate-paper",
    openBtn: "Open / Create",
    opening: "Opening…",
    current: "Current project",
    recent: "Recent projects",
    lastOpened: "Last opened",
    events: "events",
    chainOk: "chain checked",
    chainReview: "chain needs review",
    chainIdle: "not yet checked",
    emptyHint: "Open or create a project to see its event-chain status here. Below are illustrative projects.",
    templates: "Templates",
    tplNote: "Illustrative templates · sets up a workflow when picked (coming soon)",
    tpl1: "Paper proofreading",
    tpl1s: "Citation checks · figure recompute · cross-model review",
    tpl2: "Proposal review",
    tpl2s: "Clause diff · consistency check · evidence trail",
    tpl3: "Codebase analysis",
    tpl3s: "Structure map · dependency audit · change tracing",
    ill1: "2026-climate-paper",
    ill2: "grant-proposal-V3",
    ill3: "axiom3-code-audit",
  },
};

type Recent = { name: string; last: string; events: number; chain_ok: boolean | null };

export function ProjectsView({ lang }: { lang: Lang }) {
  const s = S[lang];
  const [current, setCurrent] = useState<ProjectStatus | null>(null);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);

  const recent: Recent[] = [
    { name: s.ill1, last: "2h", events: 214, chain_ok: true },
    { name: s.ill2, last: "1d", events: 87, chain_ok: null },
    { name: s.ill3, last: "3d", events: 156, chain_ok: true },
  ];

  useEffect(() => {
    invoke<ProjectStatus>("project_status").then(setCurrent).catch(() => {});
  }, []);

  async function openProject() {
    const n = name.trim();
    if (!n || busy) return;
    setBusy(true);
    try {
      await invoke("open_project", { name: n });
      const st = await invoke<ProjectStatus>("project_status").catch(() => null);
      if (st) setCurrent(st);
      setName("");
    } catch { /* keep prior state; advisory only */ }
    setBusy(false);
  }

  const lamp = (ok: boolean | null) =>
    ok === true ? "is-green" : ok === false ? "is-red" : ok === null ? "is-neutral" : "is-amber";
  const lampLabel = (ok: boolean | null) =>
    ok === true ? s.chainOk : ok === null ? s.chainIdle : s.chainReview;

  return (
    <div className="ax-surface">
      <div className="ax-surface__head">
        <div>
          <h2 className="ax-surface__title">{s.title}</h2>
          <p className="ax-surface__sub">{s.sub}</p>
        </div>
        <div className="ax-surface__actions">
          <button className="ax-btn ax-btn--primary" onClick={() => (document.querySelector(".ax-input") as HTMLInputElement)?.focus()}>
            <Plus size={15} /> {s.newBtn}
          </button>
        </div>
      </div>

      <div className="ax-card ax-card--pad">
        <div className="ax-block-label">{s.openLabel}</div>
        <div className="ax-toolrow">
          <input
            className="ax-input"
            placeholder={s.namePh}
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") openProject(); }}
          />
          <button className="ax-btn ax-btn--primary" onClick={openProject} disabled={busy || !name.trim()}>
            <FolderOpen size={15} /> {busy ? s.opening : s.openBtn}
          </button>
        </div>
      </div>

      <div className="ax-sec-label">{current ? s.current : s.recent}</div>
      <div className="ax-grid">
        {current ? (
          <div className="ax-card ax-card--pad">
            <div className="ax-card__head">
              <div className="ax-card__title"><FolderOpen size={16} /> {current.project_id}</div>
              <span className={`ax-lamp ${lamp(current.chain_ok)}`} />
            </div>
            <div className="ax-card__body">
              <div className="ax-stat">
                <div className="ax-stat__num">{current.event_count} <small>{s.events}</small></div>
                <div className="ax-stat__label">{lampLabel(current.chain_ok)}</div>
              </div>
              <div className="ax-row">
                <div className="ax-row__main">
                  <div className="ax-row__title">{current.chain_head ? current.chain_head.slice(0, 12) : "—"}</div>
                  <div className="ax-row__sub" style={{ wordBreak: "break-all" }}>{current.root}</div>
                </div>
                <div className="ax-row__meta"><FileCheck size={14} /></div>
              </div>
            </div>
          </div>
        ) : (
          recent.map((r) => (
            <div className="ax-card ax-card--pad" key={r.name}>
              <div className="ax-card__head">
                <div className="ax-card__title"><FolderOpen size={16} /> {r.name}</div>
                <span className={`ax-lamp ${lamp(r.chain_ok)}`} />
              </div>
              <div className="ax-card__body">
                <div className="ax-row__sub">{s.lastOpened} · {r.last} · {r.events} {s.events}</div>
                <div className="ax-badge">{lampLabel(r.chain_ok)}</div>
              </div>
            </div>
          ))
        )}
      </div>
      {!current && (
        <div className="ax-hint">{s.emptyHint}</div>
      )}

      <div className="ax-sec-label">{s.templates}</div>
      <div className="ax-grid">
        {[
          { t: s.tpl1, sub: s.tpl1s, Icon: FileCheck },
          { t: s.tpl2, sub: s.tpl2s, Icon: ShieldCheck },
          { t: s.tpl3, sub: s.tpl3s, Icon: Boxes },
        ].map(({ t, sub, Icon }) => (
          <div className="ax-card ax-card--pad" key={t}>
            <div className="ax-card__head">
              <div className="ax-card__title"><Icon size={16} /> {t}</div>
              <BookOpen size={14} />
            </div>
            <div className="ax-card__body">
              <div className="ax-row__sub">{sub}</div>
            </div>
          </div>
        ))}
      </div>
      <div className="ax-hint">{s.tplNote}</div>
    </div>
  );
}
