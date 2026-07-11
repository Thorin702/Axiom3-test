// 笔记本 — a live Python kernel (Claude Science's compute layer). Cells run against a
// PERSISTENT kernel (variables/dataframes/models stay in memory), so a figure or table is
// COMPUTED from real data, not asserted. Output = stdout + the last expression's repr +
// matplotlib figures (rendered inline). This is where "real work" happens.
//
// NOTE: a kernel runs arbitrary Python — that is the point, so it is not a security
// boundary. Credentials are scrubbed from the kernel env; run only code you trust.
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Play, Plus, RotateCcw, Trash2, TerminalSquare } from "lucide-react";
import type { Lang } from "../i18n";

type CellResult = { ok: boolean; stdout: string; error: string | null; figures: string[] };
type Cell = { id: string; code: string; result?: CellResult; running: boolean };
type KStatus = { running: boolean; python: string | null; sci_ready: boolean };

const S = {
  zh: {
    title: "笔记本", sub: "一个常驻的 Python 内核 —— 变量与数据在单元格之间保留,图表由真实数据算出,而非声称。",
    run: "运行", add: "新单元格", reset: "重启内核", clear: "清空", del: "删除单元格",
    ph: "在此写 Python… (⌘/Ctrl + Enter 运行)",
    noPy: "未检测到 Python 3 —— 请先安装。", live: "内核在线", idle: "内核未启动(运行任一单元格即启动)",
    running: "运行中…", empty: "还没有单元格。",
    warn: "内核执行任意 Python,不是安全沙箱;凭据已从环境剔除。仅运行你信任的代码。",
    tip: "试试:import matplotlib.pyplot as plt; plt.plot([1,4,9,16]); 会内联出图。",
    envMissing: "科学环境(numpy · pandas · matplotlib)尚未就绪 —— 绘图需要它。",
    envReady: "科学环境就绪", prepare: "准备环境", preparing: "正在安装 numpy · pandas · matplotlib…(约一分钟)",
    prepFail: "环境准备失败",
  },
  en: {
    title: "Notebook", sub: "A persistent Python kernel — variables and data stay in memory across cells, so figures are COMPUTED from real data, not asserted.",
    run: "Run", add: "Add cell", reset: "Restart kernel", clear: "Clear", del: "Delete cell",
    ph: "Write Python here… (⌘/Ctrl + Enter to run)",
    noPy: "Python 3 not found — please install it.", live: "kernel live", idle: "kernel idle (runs on first cell)",
    running: "running…", empty: "No cells yet.",
    warn: "The kernel runs arbitrary Python — it is not a security sandbox; credentials are scrubbed from its env. Run only code you trust.",
    tip: "Try: import matplotlib.pyplot as plt; plt.plot([1,4,9,16]) — it renders inline.",
    envMissing: "The scientific stack (numpy · pandas · matplotlib) isn't ready yet — plotting needs it.",
    envReady: "scientific stack ready", prepare: "Prepare environment", preparing: "Installing numpy · pandas · matplotlib… (~a minute)",
    prepFail: "Environment setup failed",
  },
};

let seq = 0;
const uid = () => `cell-${seq++}`;

export function NotebookView({ lang }: { lang: Lang }) {
  const s = S[lang];
  const [cells, setCells] = useState<Cell[]>(() => {
    try {
      const saved: string[] = JSON.parse(localStorage.getItem("axiom3.nbcells") || "[]");
      if (saved.length) return saved.map((code) => ({ id: uid(), code, running: false }));
    } catch { /* ignore */ }
    return [{ id: uid(), code: "", running: false }];
  });
  const [py, setPy] = useState<string | null | undefined>(undefined);
  const [sci, setSci] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const [prepErr, setPrepErr] = useState<string | null>(null);
  const busy = useRef(false);

  useEffect(() => {
    invoke<KStatus>("kernel_status")
      .then((st) => { setPy(st.python); setSci(!!st.sci_ready); })
      .catch(() => setPy(null));
  }, []);

  async function prepareEnv() {
    if (preparing) return;
    setPreparing(true);
    setPrepErr(null);
    try {
      await invoke<string>("kernel_prepare");
      await invoke("kernel_reset").catch(() => {}); // fresh kernel picks up the new packages
      setCells((cs) => cs.map((c) => ({ ...c, result: undefined })));
      const st = await invoke<KStatus>("kernel_status");
      setPy(st.python); setSci(!!st.sci_ready);
    } catch (e) {
      setPrepErr(String(e));
    } finally {
      setPreparing(false);
    }
  }
  useEffect(() => {
    localStorage.setItem("axiom3.nbcells", JSON.stringify(cells.map((c) => c.code)));
  }, [cells]);

  async function runCell(id: string) {
    if (busy.current) return; // one cell at a time (the kernel serialises anyway)
    const cell = cells.find((c) => c.id === id);
    if (!cell || !cell.code.trim()) return;
    busy.current = true;
    setCells((cs) => cs.map((c) => (c.id === id ? { ...c, running: true } : c)));
    try {
      const r = await invoke<CellResult>("kernel_exec", { code: cell.code });
      setCells((cs) => cs.map((c) => (c.id === id ? { ...c, result: r, running: false } : c)));
    } catch (e) {
      setCells((cs) => cs.map((c) => (c.id === id ? { ...c, result: { ok: false, stdout: "", error: String(e), figures: [] }, running: false } : c)));
    } finally {
      busy.current = false;
      invoke<KStatus>("kernel_status").then((st) => { setPy(st.python); setSci(!!st.sci_ready); }).catch(() => {});
    }
  }

  function setCode(id: string, code: string) {
    setCells((cs) => cs.map((c) => (c.id === id ? { ...c, code } : c)));
  }
  function addCell(after?: string) {
    setCells((cs) => {
      const nc = { id: uid(), code: "", running: false };
      if (!after) return [...cs, nc];
      const i = cs.findIndex((c) => c.id === after);
      return [...cs.slice(0, i + 1), nc, ...cs.slice(i + 1)];
    });
  }
  function delCell(id: string) {
    setCells((cs) => (cs.length <= 1 ? [{ id: uid(), code: "", running: false }] : cs.filter((c) => c.id !== id)));
  }
  async function resetKernel() {
    await invoke("kernel_reset").catch(() => {});
    setCells((cs) => cs.map((c) => ({ ...c, result: undefined })));
    setPy(undefined);
    invoke<KStatus>("kernel_status").then((st) => { setPy(st.python); setSci(!!st.sci_ready); }).catch(() => {});
  }

  return (
    <div className="ax-surface">
      <div className="ax-surface__head">
        <div>
          <h2 className="ax-surface__title"><TerminalSquare size={18} style={{ marginRight: 8, verticalAlign: "-3px" }} />{s.title}</h2>
          <p className="ax-surface__sub">{s.sub}</p>
        </div>
        <div className="ax-surface__actions">
          <span className="ax-badge">
            <span className={`ax-lamp ${py ? "is-green" : "is-neutral"}`} />
            {py ? s.live : py === null ? s.noPy : s.idle}
          </span>
          {sci && (
            <span className="ax-badge" title={s.envReady}>
              <span className="ax-lamp is-green" /> numpy · pandas · matplotlib
            </span>
          )}
          <button className="ax-btn" onClick={resetKernel} title={s.reset}><RotateCcw size={14} /> {s.reset}</button>
        </div>
      </div>

      {py !== null && !sci && (
        <div className="ax-nb__env">
          <span className="ax-lamp is-neutral" />
          <span className="ax-nb__envmsg">{preparing ? s.preparing : s.envMissing}</span>
          <button className="ax-btn ax-btn--primary" onClick={prepareEnv} disabled={preparing}>
            {preparing ? s.preparing : s.prepare}
          </button>
        </div>
      )}
      {prepErr && <p className="ax-nb__enverr">{s.prepFail}: {prepErr}</p>}

      <p className="ax-hint" style={{ marginBottom: 12 }}>{s.warn}</p>

      <div className="ax-nb">
        {cells.map((c, i) => (
          <div className="ax-nbcell" key={c.id}>
            <div className="ax-nbcell__gutter">
              <span className="ax-nbcell__n">[{i + 1}]</span>
              <button className="ax-iconbtn" onClick={() => runCell(c.id)} disabled={c.running || !c.code.trim()} title={s.run} aria-label={s.run}>
                <Play size={13} aria-hidden />
              </button>
              <button className="ax-iconbtn" onClick={() => delCell(c.id)} title={s.del} aria-label={s.del}>
                <Trash2 size={12} aria-hidden />
              </button>
            </div>
            <div className="ax-nbcell__main">
              <textarea
                className="ax-nbcell__code"
                value={c.code}
                spellCheck={false}
                placeholder={i === 0 ? `${s.ph}\n\n# ${s.tip}` : s.ph}
                rows={Math.min(16, Math.max(2, c.code.split("\n").length + 1))}
                onChange={(e) => setCode(c.id, e.currentTarget.value)}
                onKeyDown={(e) => {
                  if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
                    e.preventDefault();
                    runCell(c.id);
                  }
                }}
              />
              {c.running && <div className="ax-nbcell__running">{s.running}</div>}
              {c.result && (
                <div className={`ax-nbcell__out ${c.result.ok ? "" : "is-err"}`}>
                  {c.result.stdout && <pre className="ax-nbcell__stdout">{c.result.stdout}</pre>}
                  {c.result.figures.map((f, k) => (
                    <img key={k} className="ax-nbcell__fig" src={`data:image/png;base64,${f}`} alt="figure" />
                  ))}
                  {c.result.error && <pre className="ax-nbcell__errtrace">{c.result.error}</pre>}
                </div>
              )}
            </div>
          </div>
        ))}
      </div>

      <button className="ax-btn" style={{ marginTop: 10 }} onClick={() => addCell()}>
        <Plus size={14} /> {s.add}
      </button>
    </div>
  );
}
