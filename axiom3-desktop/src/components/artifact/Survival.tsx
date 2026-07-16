// Native Kaplan–Meier survival curve: the clinical-standard step function Chart.js cannot draw.
// Each group is a monotone-decreasing STAIRCASE (never smoothed — smoothing a KM curve invents
// survival between events), with censoring shown as small ticks on the plateau, an aligned
// number-at-risk table below, and log-rank p / HR[95% CI] annotations when supplied.
//
// Provenance is REQUIRED for a survival figure (these numbers are patient outcomes): when the
// spec omits it the whole figure is flagged 示意·非实测 (corner badge + caption note) rather than
// being passed off as measured — we never fabricate the missing source, only surface its absence.
//
// Spec (```survival JSON):
//   {"title"?, "xlabel"?, "ylabel"?, "provenance"?, "percent"?: boolean,
//    "groups":[{"name":string,
//               "steps":[[t:number, survival:0..1], ...],   // [event time, survival AFTER the drop]
//               "censors"?:[t, ...],                         // censoring times (ticks on the curve)
//               "atRisk"?:[[t:number, n:number], ...]}, ...],// number at risk (step function of t)
//    "logrankP"?:number,
//    "hr"?:{"est":number,"lo":number,"hi":number},          // hazard ratio + 95% CI
//    "medianSurvival"?:[{group:string, median:number}]}
// Every field except groups[*].name/steps is OPTIONAL; absent fields degrade gracefully.
// The curve never extrapolates past a group's last step time.

import { catColor, isDark, provenanceTag, type Provenance, STROKE, ALPHA, FONT } from "./theme";

type Step = [number, number];
type AtRisk = [number, number];
type Group = { name: string; steps: Step[]; censors?: number[]; atRisk?: AtRisk[] };
type HR = { est: number; lo: number; hi: number };
type Median = { group: string; median: number };
type Spec = {
  title?: string;
  xlabel?: string;
  ylabel?: string;
  provenance?: Provenance;
  percent?: boolean;
  groups: Group[];
  logrankP?: number;
  hr?: HR;
  medianSurvival?: Median[];
};

// Line-style redundancy so groups stay distinguishable without relying on hue (CVD-safe).
const DASH = ["", "6 4", "2 3", "9 3 2 3", "4 2 1 2", "12 4"];

function fmtNum(v: number): string {
  if (!Number.isFinite(v)) return "—";
  const a = Math.abs(v);
  if (a === 0) return "0";
  if (a >= 100) return v.toFixed(0);
  if (a >= 10) return v.toFixed(1);
  if (a >= 1) return v.toFixed(2);
  return v.toFixed(3);
}
function fmtP(p: number): string {
  if (!Number.isFinite(p)) return "—";
  if (p < 0.001) return "P<0.001";
  if (p < 0.01) return `P=${p.toFixed(4)}`;
  return `P=${p.toFixed(3)}`;
}
function trimZeros(s: string): string {
  return s.includes(".") ? s.replace(/\.?0+$/, "") : s;
}
// CJK-aware glyph width (~11px CJK / ~6.4px latin) at the legend/label font size.
function cw(s: string): number {
  let w = 0;
  for (const c of s) w += /[⺀-鿿＀-￯　-〿]/.test(c) ? 11 : 6.4;
  return w;
}

function niceNum(range: number, round: boolean): number {
  const exp = Math.floor(Math.log10(range || 1));
  const f = range / Math.pow(10, exp);
  let nf: number;
  if (round) nf = f < 1.5 ? 1 : f < 3 ? 2 : f < 7 ? 5 : 10;
  else nf = f <= 1 ? 1 : f <= 2 ? 2 : f <= 5 ? 5 : 10;
  return nf * Math.pow(10, exp);
}
function linTicks(min: number, max: number, count: number): number[] {
  if (!(max > min)) return [min];
  const step = niceNum(niceNum(max - min, false) / Math.max(1, count - 1), true);
  const start = Math.ceil(min / step) * step;
  const out: number[] = [];
  for (let v = start; v <= max + step * 0.5; v += step) out.push(Math.round(v / step) * step);
  return out;
}

// Step-function lookup: value carried forward from the largest key <= t (KM plateau semantics).
// Before the first key the survival plateau is 1 (everyone alive / at risk).
function stepValueAt(pairs: [number, number][], t: number, before: number): number {
  let v = before;
  for (const [k, val] of pairs) {
    if (k <= t) v = val;
    else break;
  }
  return v;
}

export function Survival({ spec, caption }: { spec: string; caption?: string }) {
  let d: Spec;
  try {
    d = JSON.parse(spec.trim());
  } catch (e) {
    return <div className="ax-fig__err">生存曲线规范不是有效 JSON · {String((e as Error).message)}</div>;
  }
  if (!d || typeof d !== "object") return <div className="ax-fig__err">生存曲线规范为空或格式不对</div>;
  if (!Array.isArray(d.groups))
    return <div className="ax-fig__err">生存曲线需要 groups 数组(每组含 name / steps[[t, survival], …])</div>;

  const dark = isDark();

  // Validate + normalize groups: keep only finite [t, survival] pairs, clamp survival to [0,1],
  // sort by time, and drop groups with no usable steps rather than throwing.
  const groups = d.groups
    .filter((g): g is Group => !!g && typeof g.name === "string" && Array.isArray(g.steps))
    .map((g) => {
      const steps = g.steps
        .filter((s): s is Step => Array.isArray(s) && s.length >= 2 && Number.isFinite(s[0]) && Number.isFinite(s[1]))
        .map((s) => [s[0], Math.max(0, Math.min(1, s[1]))] as Step)
        .sort((a, b) => a[0] - b[0]);
      const censors = Array.isArray(g.censors) ? g.censors.filter((t) => Number.isFinite(t)) : [];
      const atRisk = Array.isArray(g.atRisk)
        ? g.atRisk
            .filter((a): a is AtRisk => Array.isArray(a) && a.length >= 2 && Number.isFinite(a[0]) && Number.isFinite(a[1]))
            .sort((a, b) => a[0] - b[0])
        : [];
      return { name: g.name, steps, censors, atRisk };
    })
    .filter((g) => g.steps.length > 0);

  if (groups.length === 0) return <div className="ax-fig__err">生存曲线无有效数据(每组 steps 需含 [时间, 生存概率] 数对)</div>;

  // Provenance gate: patient-outcome curves must say where they came from. Missing / non-"ok" → 示意.
  const provTag = provenanceTag(d.provenance);
  const schematic = !provTag || provTag.kind !== "ok";

  const percent = d.percent === true;
  const anyRisk = groups.some((g) => g.atRisk.length > 0);
  const medians = Array.isArray(d.medianSurvival)
    ? d.medianSurvival.filter((m): m is Median => !!m && typeof m.group === "string" && Number.isFinite(m.median))
    : [];

  // ── x domain: 0 → latest step time across groups (never extrapolate past last event) ──
  let tMax = 0;
  for (const g of groups) {
    tMax = Math.max(tMax, g.steps[g.steps.length - 1][0]);
    for (const [t] of g.atRisk) tMax = Math.max(tMax, t);
  }
  if (!(tMax > 0)) tMax = 1;
  const tMin = 0;
  const xticks = linTicks(tMin, tMax, 6).filter((t) => t >= tMin - 1e-9 && t <= tMax + 1e-9);
  // Risk-table columns: the x-tick times, with 0 guaranteed as the first column.
  const riskCols = xticks.includes(0) ? xticks : [0, ...xticks];

  // ── layout ──
  const padL = 52;
  const padR = 16;
  const padT = d.title ? 30 : 14;
  const plotW = 440;
  const plotH = 260;
  const x0 = padL;
  const y0 = padT;
  const plotBottom = y0 + plotH;
  const axisLabelH = 30; // x-axis ticks + xlabel
  const riskHeaderH = anyRisk ? 18 : 0;
  const riskRowH = 17;
  const riskH = anyRisk ? riskHeaderH + groups.length * riskRowH + 8 : 0;
  const w = x0 + plotW + padR;
  const h = plotBottom + axisLabelH + riskH + 8;

  const xs = (t: number) => x0 + ((t - tMin) / (tMax - tMin || 1)) * plotW;
  const ys = (s: number) => y0 + (1 - Math.max(0, Math.min(1, s))) * plotH;

  const yticks = [0, 0.25, 0.5, 0.75, 1];
  const yLabel = (s: number) => (percent ? `${Math.round(s * 100)}` : trimZeros(s.toFixed(2)));
  const ylabel = d.ylabel || (percent ? "生存率 (%)" : "生存概率");
  const xlabel = d.xlabel || "时间";

  // Build a step-after staircase path (hold y, advance x, then drop) — the KM convention.
  function stairPath(steps: Step[]): string {
    if (steps.length === 0) return "";
    let dpth = `M ${xs(steps[0][0]).toFixed(2)} ${ys(steps[0][1]).toFixed(2)}`;
    for (let i = 1; i < steps.length; i++) {
      const [t, s] = steps[i];
      const prevS = steps[i - 1][1];
      dpth += ` L ${xs(t).toFixed(2)} ${ys(prevS).toFixed(2)} L ${xs(t).toFixed(2)} ${ys(s).toFixed(2)}`;
    }
    return dpth;
  }

  // Annotation lines (log-rank p, HR[95% CI]) — drawn in a small box, low-left of the plot.
  const statLines: string[] = [];
  if (Number.isFinite(d.logrankP)) statLines.push(`log-rank ${fmtP(d.logrankP as number)}`);
  if (d.hr && Number.isFinite(d.hr.est) && Number.isFinite(d.hr.lo) && Number.isFinite(d.hr.hi))
    statLines.push(`HR ${fmtNum(d.hr.est)} (95% CI ${fmtNum(d.hr.lo)}–${fmtNum(d.hr.hi)})`);

  const legendMaxW = Math.max(...groups.map((g) => cw(g.name))) + 30;
  const legendX = x0 + plotW - legendMaxW - 4;

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg viewBox={`0 0 ${w} ${h}`} width={w} height={h} role="img" aria-label={caption || d.title || "kaplan-meier survival curve"} style={{ maxWidth: "100%" }}>
          {d.title && (
            <text x={padL} y={18} fontSize={FONT.title} fontWeight="600" fill="var(--ax-ink)">
              {d.title}
            </text>
          )}

          {/* corner badge when provenance is absent — the figure is illustrative, not measured */}
          {schematic && (
            <g>
              <rect x={w - 118} y={2} width={112} height={15} rx={3} fill="var(--ax-surface)" stroke="var(--ax-line-soft)" strokeWidth="0.5" opacity={0.9} />
              <text x={w - 62} y={12.5} fontSize={FONT.min} textAnchor="middle" fill="var(--ax-ink-faint)">
                示意 · 非实测数据
              </text>
            </g>
          )}

          {/* horizontal gridlines */}
          {yticks.map((t, i) => (
            <line key={`gy${i}`} x1={x0} y1={ys(t)} x2={x0 + plotW} y2={ys(t)} stroke="var(--ax-line-soft)" strokeWidth={STROKE.grid} />
          ))}

          {/* median reference at 0.5 when medians are provided */}
          {medians.length > 0 && (
            <line x1={x0} y1={ys(0.5)} x2={x0 + plotW} y2={ys(0.5)} stroke="var(--ax-ink-faint)" strokeWidth={STROKE.ref} strokeDasharray="3 3" opacity={ALPHA.ref} />
          )}

          {/* axis frame */}
          <line x1={x0} y1={y0} x2={x0} y2={plotBottom} stroke="var(--ax-line)" strokeWidth={STROKE.axis} />
          <line x1={x0} y1={plotBottom} x2={x0 + plotW} y2={plotBottom} stroke="var(--ax-line)" strokeWidth={STROKE.axis} />

          {/* y ticks + labels */}
          {yticks.map((t, i) => (
            <g key={`yt${i}`}>
              <line x1={x0 - 4} y1={ys(t)} x2={x0} y2={ys(t)} stroke="var(--ax-line)" strokeWidth={STROKE.axis} />
              <text x={x0 - 7} y={ys(t) + 3} fontSize={FONT.tick} textAnchor="end" fill="var(--ax-ink-faint)">
                {yLabel(t)}
              </text>
            </g>
          ))}
          <text x={15} y={y0 + plotH / 2} fontSize={FONT.axis} textAnchor="middle" fill="var(--ax-ink-soft)" transform={`rotate(-90 15 ${y0 + plotH / 2})`}>
            {ylabel}
          </text>

          {/* x ticks + labels */}
          {xticks.map((t, i) => (
            <g key={`xt${i}`}>
              <line x1={xs(t)} y1={plotBottom} x2={xs(t)} y2={plotBottom + 4} stroke="var(--ax-line)" strokeWidth={STROKE.axis} />
              <text x={xs(t)} y={plotBottom + 15} fontSize={FONT.tick} textAnchor="middle" fill="var(--ax-ink-faint)">
                {trimZeros(fmtNum(t))}
              </text>
            </g>
          ))}
          <text x={x0 + plotW / 2} y={plotBottom + axisLabelH - 2} fontSize={FONT.axis} textAnchor="middle" fill="var(--ax-ink-soft)">
            {xlabel}
          </text>

          {/* median vertical drops per group */}
          {medians.map((m, i) => {
            const gi = groups.findIndex((g) => g.name === m.group);
            if (gi < 0 || m.median < tMin || m.median > tMax) return null;
            const color = catColor(gi, dark);
            return (
              <line key={`med${i}`} x1={xs(m.median)} y1={ys(0.5)} x2={xs(m.median)} y2={plotBottom} stroke={color} strokeWidth={STROKE.ref} strokeDasharray="4 3" opacity={0.7} />
            );
          })}

          {/* KM staircases + censoring ticks */}
          {groups.map((g, gi) => {
            const color = catColor(gi, dark);
            const dash = DASH[gi % DASH.length];
            return (
              <g key={`grp${gi}`}>
                <path d={stairPath(g.steps)} fill="none" stroke={color} strokeWidth={STROKE.emphasis} strokeDasharray={dash || undefined} strokeLinejoin="miter">
                  <title>{`${g.name} — Kaplan–Meier`}</title>
                </path>
                {g.censors.map((ct, ci) => {
                  if (ct < tMin || ct > tMax) return null;
                  const sv = stepValueAt(g.steps as [number, number][], ct, 1);
                  const cx = xs(ct);
                  const cyv = ys(sv);
                  return (
                    <line key={`cen${gi}-${ci}`} x1={cx} y1={cyv - 4} x2={cx} y2={cyv + 4} stroke={color} strokeWidth={STROKE.main}>
                      <title>{`${g.name} — 删失 @ ${trimZeros(fmtNum(ct))}`}</title>
                    </line>
                  );
                })}
              </g>
            );
          })}

          {/* legend: color swatch + line style + name (never colour-alone) */}
          {groups.map((g, gi) => {
            const color = catColor(gi, dark);
            const dash = DASH[gi % DASH.length];
            const ly = y0 + 8 + gi * 16;
            return (
              <g key={`lg${gi}`}>
                <line x1={legendX} y1={ly} x2={legendX + 22} y2={ly} stroke={color} strokeWidth={STROKE.emphasis} strokeDasharray={dash || undefined} />
                <text x={legendX + 28} y={ly + 3.5} fontSize={FONT.tick} fill="var(--ax-ink-soft)">
                  {g.name}
                </text>
              </g>
            );
          })}

          {/* stats box (log-rank p, HR[95% CI]) — low-left inside the plot */}
          {statLines.length > 0 && (
            <g>
              {statLines.map((s, i) => (
                <text key={`st${i}`} x={x0 + 8} y={plotBottom - 8 - (statLines.length - 1 - i) * 13} fontSize={FONT.tick} fill="var(--ax-ink-soft)" style={{ fontVariantNumeric: "tabular-nums" }}>
                  {s}
                </text>
              ))}
            </g>
          )}

          {/* number-at-risk table (rows aligned to the x-tick columns) */}
          {anyRisk && (
            <g>
              <text x={x0} y={plotBottom + axisLabelH + 10} fontSize={FONT.min} fill="var(--ax-ink-faint)">
                风险人数 · No. at risk
              </text>
              {groups.map((g, gi) => {
                const color = catColor(gi, dark);
                const ry = plotBottom + axisLabelH + riskHeaderH + gi * riskRowH + 8;
                return (
                  <g key={`risk${gi}`}>
                    <circle cx={x0 - 6} cy={ry - 3} r={2.6} fill={color} />
                    {riskCols.map((t, ci) => {
                      if (g.atRisk.length === 0) {
                        return ci === 0 ? (
                          <text key={`rc${gi}-${ci}`} x={xs(t)} y={ry} fontSize={FONT.min} textAnchor="middle" fill="var(--ax-ink-faint)">
                            —
                          </text>
                        ) : null;
                      }
                      // Before the first atRisk time there is no reported count → blank.
                      if (t < g.atRisk[0][0] - 1e-9) return null;
                      const n = stepValueAt(g.atRisk as [number, number][], t, g.atRisk[0][1]);
                      return (
                        <text key={`rc${gi}-${ci}`} x={xs(t)} y={ry} fontSize={FONT.min} textAnchor="middle" fill="var(--ax-ink-soft)" style={{ fontVariantNumeric: "tabular-nums" }}>
                          {Math.round(n)}
                        </text>
                      );
                    })}
                  </g>
                );
              })}
            </g>
          )}
        </svg>
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">生存曲线 · Kaplan–Meier</span> {caption || d.title || ""}
        {schematic ? (
          <span className="ax-fig__note"> · 示意 · 非实测数据(生存曲线未标注被引/计算来源,数字仅为示意)</span>
        ) : (
          provTag?.kind === "ok" && <span className="ax-fig__note"> · 数据来源:{provTag.text}</span>
        )}
      </figcaption>
    </figure>
  );
}
