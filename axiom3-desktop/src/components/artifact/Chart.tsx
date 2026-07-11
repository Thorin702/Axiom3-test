// Native data-figure rendering: a ```chart JSON block → a scatter/UMAP, bar, or line chart,
// drawn client-side with Chart.js (lazy-loaded). Styling follows the data-viz skill: the
// validated CVD-safe categorical palette, fixed hue order (never cycled), text in ink
// tokens (never the series color), one axis, a legend for ≥2 groups.
//
// Spec (the agent emits one of):
//   {"type":"scatter","title","xlabel","ylabel","points":[{"x","y","group"?}]}
//   {"type":"bar","title","xlabel","ylabel","data":[{"label","value","group"?}]}
//   {"type":"line","title","xlabel","ylabel","series":[{"name","points":[[x,y]...]}]}
import { useEffect, useRef, useState } from "react";

// data-viz reference categorical palette (validated), light / dark by slot order.
const CAT_LIGHT = ["#2a78d6", "#1baf7a", "#eda100", "#008300", "#4a3aa7", "#e34948", "#e87ba4", "#eb6834"];
const CAT_DARK = ["#3987e5", "#199e70", "#c98500", "#008300", "#9085e9", "#e66767", "#d55181", "#d95926"];

function isDark(): boolean {
  const t = document.documentElement.dataset.theme;
  if (t === "dark") return true;
  if (t === "light") return false;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
}
function tokens() {
  const cs = getComputedStyle(document.documentElement);
  const v = (n: string, f: string) => (cs.getPropertyValue(n).trim() || f);
  return {
    ink: v("--ax-ink", "#1a1a1a"),
    soft: v("--ax-ink-soft", "#555"),
    faint: v("--ax-ink-faint", "#999"),
    line: v("--ax-line", "#e4e3dd"),
    cat: isDark() ? CAT_DARK : CAT_LIGHT,
  };
}

type Spec = {
  type: "scatter" | "bar" | "line";
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
};

// Coerce the various bar shapes agents emit into the canonical [{label, value}].
function barRows(spec: Spec): { label: string; value: number }[] {
  const d = spec.data;
  if (Array.isArray(d) && d.length && typeof d[0] === "object" && d[0] !== null && "value" in d[0]) {
    return d.map((r: { label?: unknown; value: number }) => ({ label: String(r.label ?? ""), value: Number(r.value) }));
  }
  // {x:[...],y:[...]} or {labels:[...],values:[...]} — at the top level or nested under data.
  const src = (d && typeof d === "object" && !Array.isArray(d)) ? d : spec;
  const labels = src.labels ?? src.x ?? [];
  const vals = src.values ?? src.y ?? [];
  if (Array.isArray(labels) && Array.isArray(vals) && vals.length) {
    return vals.map((v: number, i: number) => ({ label: String(labels[i] ?? i + 1), value: Number(v) }));
  }
  return [];
}

function build(spec: Spec, tk: ReturnType<typeof tokens>) {
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
      backgroundColor: tk.cat[i % tk.cat.length],
      pointRadius: 3,
      pointHoverRadius: 5,
    }));
    if (single) base.plugins.legend.display = false;
    return { type: "scatter", data: { datasets }, options: base };
  }

  if (spec.type === "bar") {
    const rows = barRows(spec);
    return {
      type: "bar",
      data: {
        labels: rows.map((r) => r.label),
        datasets: [{ label: spec.ylabel || "value", data: rows.map((r) => r.value), backgroundColor: rows.map((_, i) => tk.cat[i % tk.cat.length]), borderRadius: 4, borderSkipped: false }],
      },
      options: { ...base, plugins: { ...base.plugins, legend: { display: false } }, scales: { ...base.scales, x: { ...base.scales.x, grid: { display: false } } } },
    };
  }

  // line
  const datasets = (spec.series ?? []).map((s, i) => ({
    label: s.name,
    data: s.points.map(([x, y]) => ({ x, y })),
    borderColor: tk.cat[i % tk.cat.length],
    backgroundColor: tk.cat[i % tk.cat.length],
    borderWidth: 2,
    pointRadius: 2,
    tension: 0.2,
  }));
  if (datasets.length <= 1) base.plugins.legend.display = false;
  return { type: "line", data: { datasets }, options: base };
}

export function Chart({ spec: raw, caption }: { spec: string; caption?: string }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    let chart: { destroy: () => void } | null = null;
    let alive = true;
    (async () => {
      let spec: Spec;
      try {
        spec = JSON.parse(raw.trim());
      } catch {
        setErr("chart spec is not valid JSON");
        return;
      }
      if (!["scatter", "bar", "line"].includes(spec.type)) {
        setErr(`unsupported chart type: ${spec.type}`);
        return;
      }
      try {
        const { Chart: ChartJS, registerables } = await import("chart.js");
        ChartJS.register(...registerables);
        if (!alive || !ref.current) return;
        const cfg = build(spec, tokens());
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        chart = new ChartJS(ref.current, cfg as any);
      } catch (e) {
        setErr(String((e as Error)?.message || e));
      }
    })();
    return () => {
      alive = false;
      chart?.destroy();
    };
  }, [raw]);

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame ax-fig__chartframe">
        {err ? <div className="ax-fig__err">图表无法渲染 · {err}</div> : <canvas ref={ref} />}
      </div>
      {caption && <figcaption className="ax-fig__cap"><span className="ax-fig__kind">图表 · chart</span> {caption}</figcaption>}
    </figure>
  );
}
