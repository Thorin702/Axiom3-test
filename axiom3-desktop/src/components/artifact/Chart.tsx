// Native data-figure rendering: a ```chart JSON block → a scatter/UMAP, bar, or line chart,
// drawn client-side with Chart.js (lazy-loaded). Styling follows the data-viz skill: the
// validated CVD-safe categorical palette, fixed hue order (never cycled), text in ink
// tokens (never the series color), one axis, a legend for ≥2 groups.
//
// Honesty rails (publication rubric):
//   • bar axis is FORCED to a zero baseline (never a truncated / "broken" axis) — bars encode
//     magnitude by length, so a non-zero baseline lies about ratios. Negative data keeps 0 in view.
//   • pie / donut are REFUSED and auto-rewritten to a 100%-stacked proportion bar (length beats
//     angle for part-to-whole), with a caption note. If the data can't be read, we ignore quietly.
//   • error bars are only drawn AND labelled when BOTH errorType and n are given — an unexplained
//     whisker is worse than none, so absent that metadata we degrade to plain bars.
//
// Spec (the agent emits one of):
//   {"type":"scatter","title","xlabel","ylabel","points":[{"x","y","group"?}]}
//   {"type":"bar","title","xlabel","ylabel","data":[{"label","value","error"?}],"errorType"?,"n"?}
//   {"type":"line","title","xlabel","ylabel","series":[{"name","points":[[x,y]...]}]}
//   {"type":"pie"|"donut","title","data":[{"label","value"}]}  → rewritten to a stacked % bar
import { useEffect, useRef, useState } from "react";
import { catColor, isDark } from "./theme";

function tokens() {
  const cs = getComputedStyle(document.documentElement);
  const v = (n: string, f: string) => (cs.getPropertyValue(n).trim() || f);
  return {
    ink: v("--ax-ink", "#1a1a1a"),
    soft: v("--ax-ink-soft", "#555"),
    faint: v("--ax-ink-faint", "#999"),
    line: v("--ax-line", "#e4e3dd"),
    dark: isDark(),
  };
}

type Spec = {
  type: "scatter" | "bar" | "line" | "pie" | "donut";
  title?: string;
  xlabel?: string;
  ylabel?: string;
  points?: { x: number; y: number; group?: string }[];
  // canonical bar shape; agents also emit {x:[],y:[]} / {labels:[],values:[]} — normalized below.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  data?: any;
  series?: { name: string; points: [number, number][] }[];
  labels?: (string | number)[];
  values?: number[];
  x?: (string | number)[];
  y?: number[];
  // optional error-bar metadata (bar only). errorType names what the whisker means (sd/sem/ci95/…),
  // n is the sample size. BOTH are required before a whisker is drawn or labelled.
  errorType?: string;
  n?: number;
};

type BarRow = { label: string; value: number; error?: number };

function numOrUndef(v: unknown): number | undefined {
  const n = Number(v);
  return v != null && v !== "" && isFinite(n) ? n : undefined;
}

// Coerce the various bar shapes agents emit into the canonical [{label, value, error?}].
function barRows(spec: Spec): BarRow[] {
  const d = spec.data;
  if (Array.isArray(d) && d.length && typeof d[0] === "object" && d[0] !== null && "value" in d[0]) {
    return d.map((r: { label?: unknown; value: number; error?: unknown }) => ({ label: String(r.label ?? ""), value: Number(r.value), error: numOrUndef(r.error) }));
  }
  // {x:[...],y:[...]} or {labels:[...],values:[...]} — at the top level or nested under data.
  const src = (d && typeof d === "object" && !Array.isArray(d)) ? d : spec;
  const labels = src.labels ?? src.x ?? [];
  const vals = src.values ?? src.y ?? [];
  const errs = src.errors ?? src.error ?? [];
  if (Array.isArray(labels) && Array.isArray(vals) && vals.length) {
    return vals.map((v: number, i: number) => ({ label: String(labels[i] ?? i + 1), value: Number(v), error: Array.isArray(errs) ? numOrUndef(errs[i]) : undefined }));
  }
  return [];
}

// Human-readable meaning of an error-bar type (drives the caption note).
function errLabel(t: string): string {
  const s = t.trim().toLowerCase();
  if (["sd", "std", "stdev", "stddev"].includes(s)) return "±1 SD";
  if (["se", "sem", "stderr"].includes(s)) return "±1 SEM";
  if (/(^|[^a-z])ci|95/.test(s)) return "95% CI";
  if (s === "iqr") return "IQR";
  if (s === "range") return "全距";
  return t.trim();
}

/* eslint-disable @typescript-eslint/no-explicit-any */
// A tiny Chart.js plugin that draws honest whiskers on a bar chart. Only wired up when the
// caller supplied errorType + n (see the bar branch), so every whisker on screen is explained.
function errorBarPlugin(rows: BarRow[], ink: string): any {
  return {
    id: "axErrorBars",
    afterDatasetsDraw(chart: any) {
      const meta = chart.getDatasetMeta(0);
      if (!meta) return;
      const y = chart.scales.y;
      const { ctx, chartArea } = chart;
      const clamp = (px: number) => Math.max(chartArea.top, Math.min(chartArea.bottom, px));
      ctx.save();
      ctx.strokeStyle = ink;
      ctx.lineWidth = 1;
      const cap = 4;
      meta.data.forEach((bar: any, i: number) => {
        const e = rows[i]?.error;
        const v = rows[i]?.value;
        if (typeof e !== "number" || !isFinite(e) || e <= 0 || typeof v !== "number") return;
        const x = bar.x;
        const hi = clamp(y.getPixelForValue(v + e));
        const lo = clamp(y.getPixelForValue(v - e));
        ctx.beginPath();
        ctx.moveTo(x, hi); ctx.lineTo(x, lo);
        ctx.moveTo(x - cap, hi); ctx.lineTo(x + cap, hi);
        ctx.moveTo(x - cap, lo); ctx.lineTo(x + cap, lo);
        ctx.stroke();
      });
      ctx.restore();
    },
  };
}
/* eslint-enable @typescript-eslint/no-explicit-any */

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type BuildResult = { cfg: any; note?: string };

function build(spec: Spec, tk: ReturnType<typeof tokens>): BuildResult {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const base: any = {
    responsive: true,
    maintainAspectRatio: false,
    plugins: {
      legend: { labels: { color: tk.soft, boxWidth: 10, boxHeight: 10, font: { size: 11 } } },
      title: spec.title ? { display: true, text: spec.title, color: tk.ink, font: { size: 13, weight: "600" } } : { display: false },
      tooltip: { enabled: true },
    },
    scales: {
      x: { title: { display: !!spec.xlabel, text: spec.xlabel, color: tk.faint }, ticks: { color: tk.faint, font: { size: 10 } }, grid: { color: tk.line } },
      y: { title: { display: !!spec.ylabel, text: spec.ylabel, color: tk.faint }, ticks: { color: tk.faint, font: { size: 10 } }, grid: { color: tk.line } },
    },
  };

  if (spec.type === "scatter") {
    const groups = [...new Set((spec.points ?? []).map((p) => p.group ?? ""))];
    const single = groups.length <= 1;
    const datasets = (single ? [""] : groups).map((g, i) => ({
      label: g || "points",
      data: (spec.points ?? []).filter((p) => (single ? true : (p.group ?? "") === g)).map((p) => ({ x: p.x, y: p.y })),
      backgroundColor: catColor(i, tk.dark),
      pointRadius: 3,
      pointHoverRadius: 5,
    }));
    if (single) base.plugins.legend.display = false;
    return { cfg: { type: "scatter", data: { datasets }, options: base } };
  }

  // Pie / donut are refused: angle is a weak visual channel for part-to-whole. Rewrite the shares
  // into a single 100%-stacked horizontal bar (length encoding). If the data can't be read → null,
  // and the caller shows a graceful error rather than a fabricated chart.
  if (spec.type === "pie" || spec.type === "donut") {
    const rows = barRows(spec).filter((r) => isFinite(r.value) && r.value > 0);
    const total = rows.reduce((s, r) => s + r.value, 0);
    if (!rows.length || total <= 0) return { cfg: null };
    const datasets = rows.map((r, i) => ({
      label: r.label || `第${i + 1}项`,
      data: [Number(((r.value / total) * 100).toFixed(2))],
      backgroundColor: catColor(i, tk.dark),
      borderWidth: 0,
    }));
    return {
      cfg: {
        type: "bar",
        data: { labels: [""], datasets },
        options: {
          ...base,
          indexAxis: "y",
          plugins: { ...base.plugins, legend: { ...base.plugins.legend, display: true } },
          scales: {
            x: { stacked: true, min: 0, max: 100, title: { display: true, text: "占比 (%)", color: tk.faint }, ticks: { color: tk.faint, font: { size: 10 } }, grid: { color: tk.line } },
            y: { stacked: true, ticks: { display: false }, grid: { display: false } },
          },
        },
      },
      note: "饼图已转为 100% 堆叠占比条(长度比角度更易判读)",
    };
  }

  if (spec.type === "bar") {
    const rows = barRows(spec);
    // Honest axis: bars encode magnitude by length, so the baseline MUST be zero — never a
    // truncated / broken axis. Negative data keeps 0 in frame (beginAtZero) but isn't clamped.
    const hasNeg = rows.some((r) => r.value < 0);
    const et = spec.errorType?.trim();
    const nOk = typeof spec.n === "number" && isFinite(spec.n) && spec.n > 0;
    // A whisker is only meaningful once we know what it is (errorType) and its n — gate on both.
    const drawErr = !!et && nOk && rows.some((r) => typeof r.error === "number" && r.error > 0);
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const cfg: any = {
      type: "bar",
      data: {
        labels: rows.map((r) => r.label),
        datasets: [{ label: spec.ylabel || "value", data: rows.map((r) => r.value), backgroundColor: rows.map((_, i) => catColor(i, tk.dark)), borderRadius: 4, borderSkipped: false }],
      },
      options: {
        ...base,
        plugins: { ...base.plugins, legend: { display: false } },
        scales: {
          ...base.scales,
          x: { ...base.scales.x, grid: { display: false } },
          y: { ...base.scales.y, beginAtZero: true, ...(hasNeg ? {} : { min: 0 }) },
        },
      },
    };
    if (drawErr) cfg.plugins = [errorBarPlugin(rows, tk.ink)];
    return { cfg, note: drawErr ? `误差条 = ${errLabel(et!)}(n=${spec.n})` : undefined };
  }

  // line
  const datasets = (spec.series ?? []).map((s, i) => ({
    label: s.name,
    data: s.points.map(([x, y]) => ({ x, y })),
    borderColor: catColor(i, tk.dark),
    backgroundColor: catColor(i, tk.dark),
    borderWidth: 2,
    pointRadius: 2,
    tension: 0.2,
  }));
  if (datasets.length <= 1) base.plugins.legend.display = false;
  // numeric x needs an explicit linear scale — Chart.js's line controller defaults x to
  // "category", which plots by index and distorts non-uniform (e.g. log-spaced) spacing.
  // Keep the default category scale when x are string labels.
  const numericX = (spec.series ?? []).every((s) => s.points.every(([x]) => typeof x === "number"));
  const opts = numericX ? { ...base, scales: { ...base.scales, x: { ...base.scales.x, type: "linear" } } } : base;
  return { cfg: { type: "line", data: { datasets }, options: opts } };
}

export function Chart({ spec: raw, caption }: { spec: string; caption?: string }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const [err, setErr] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [themeTick, setThemeTick] = useState(0);

  // rebuild the chart when the OS/app theme flips (tokens() bakes colors at build time)
  useEffect(() => {
    if (typeof window === "undefined") return;
    const bump = (): void => setThemeTick((n) => n + 1);
    const mo = new MutationObserver(bump);
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    const mq = window.matchMedia?.("(prefers-color-scheme: dark)");
    mq?.addEventListener?.("change", bump);
    return () => {
      mo.disconnect();
      mq?.removeEventListener?.("change", bump);
    };
  }, []);

  useEffect(() => {
    let chart: { destroy: () => void } | null = null;
    let alive = true;
    setErr(null);
    setNote(null);
    (async () => {
      let spec: Spec;
      try {
        spec = JSON.parse(raw.trim());
      } catch {
        setErr("chart spec is not valid JSON");
        return;
      }
      if (!["scatter", "bar", "line", "pie", "donut"].includes(spec.type)) {
        setErr(`unsupported chart type: ${spec.type}`);
        return;
      }
      try {
        const { Chart: ChartJS, registerables } = await import("chart.js");
        ChartJS.register(...registerables);
        if (!alive || !ref.current) return;
        const { cfg, note: buildNote } = build(spec, tokens());
        if (!cfg) {
          setErr("图表数据为空,无法渲染");
          return;
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        chart = new ChartJS(ref.current, cfg as any);
        if (alive) setNote(buildNote ?? null);
      } catch (e) {
        setErr(String((e as Error)?.message || e));
      }
    })();
    return () => {
      alive = false;
      chart?.destroy();
    };
  }, [raw, themeTick]);

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame ax-fig__chartframe">
        {err ? <div className="ax-fig__err">图表无法渲染 · {err}</div> : <canvas ref={ref} />}
      </div>
      {(caption || note) && <figcaption className="ax-fig__cap"><span className="ax-fig__kind">图表 · chart</span> {[caption, note].filter(Boolean).join(" · ")}</figcaption>}
    </figure>
  );
}
