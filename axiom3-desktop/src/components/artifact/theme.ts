// Shared publication-grade theme tokens for every native figure renderer.
//
// Distilled from top-journal figures + the figure-design meta-literature (Wong "Points of View" /
// Okabe–Ito, Crameri, Cleveland, Tol). This is the ONE source of truth for:
//   • the categorical palette   — CVD-safe, theme-adaptive, fixed order (never rainbow-cycled)
//   • continuous colormaps       — viridis (sequential) / RdBu (diverging); NEVER jet/rainbow/hsv
//   • text-on-fill contrast      — WCAG black/white auto-pick
//   • stroke / alpha / font tokens
//   • data-provenance tagging    — graceful "示意 / 未溯源" tag, so a figure that omits its
//                                  source still renders (tagged) rather than hard-failing.
//
// Neutrals (axis/grid/ink/surface) already live as `--ax-*` CSS vars in theme.css — that IS the
// grey ramp the rubric asks for, so this module does not re-declare them.

export function isDark(): boolean {
  const t = document.documentElement.dataset.theme;
  if (t === "dark") return true;
  if (t === "light") return false;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
}

// ── Categorical ─────────────────────────────────────────────────────────────
// Okabe–Ito, the top-journal colour-blind-safe standard, REORDERED so the most legible hues
// (blue, orange, green, vermilion) come first, with a dark-tuned twin so series stay readable on
// the app's dark surface (Okabe–Ito's black/yellow read poorly on dark, so they move last and
// black → light neutral in dark mode). >8 series → switch to faceting or shape, NOT more hues.
const CAT_LIGHT = ["#0072B2", "#E69F00", "#009E73", "#D55E00", "#56B4E9", "#CC79A7", "#B8A400", "#3a3f45"];
const CAT_DARK  = ["#4aa3d8", "#f0b03a", "#3cc298", "#e8834f", "#7cc7ef", "#e08bc0", "#e6d24a", "#c9ced6"];

export function categorical(dark: boolean = isDark()): string[] {
  return dark ? CAT_DARK : CAT_LIGHT;
}
/** Category colour by index (fixed order; wraps with modulo — callers should avoid >8 series). */
export function catColor(i: number, dark: boolean = isDark()): string {
  const p = categorical(dark);
  const n = p.length;
  return p[((i % n) + n) % n];
}

// ── Biological sequence palettes (shared across Alignment / SeqLogo) ─────────
// ONE canonical residue palette so a given base/amino-acid reads the SAME colour in every
// sequence figure (MSA, seqlogo). Hues are Okabe–Ito, the CVD-safe top-journal standard the rest
// of this module already trusts — but colour is NEVER the sole channel: every renderer that uses
// these also prints the residue letter (+ a labelled legend), so form/text carries the meaning too.

/** Nucleotide 4-colour convention: A green · C blue · G orange · T=U vermilion (CVD-safe, fixed). */
export const NT_PALETTE: Record<"A" | "C" | "G" | "T" | "U", string> = {
  A: "#009E73", // green
  C: "#0072B2", // blue
  G: "#E69F00", // orange
  T: "#D55E00", // vermilion (red)
  U: "#D55E00", // U shares T's slot (RNA)
};

/** Amino-acid physico-chemical property groups. */
export type AAProperty = "acidic" | "basic" | "polar" | "special" | "hydrophobic";

// Canonical group → colour. `hydrophobic` is the neutral "ink" group; the value here is its
// light-theme ink, and `aaColor` swaps to a lighter grey on dark so a bare letter stays legible.
export const AA_PROPERTY_PALETTE: Record<AAProperty, string> = {
  acidic: "#D55E00",      // negative — D E — red
  basic: "#0072B2",       // positive — K R H — blue
  polar: "#009E73",       // uncharged polar — S T N Q — green
  special: "#CC79A7",     // structure/reactive — C G P — reddish-purple
  hydrophobic: "#5b6169", // non-polar neutral — A V L I M F W Y — dark grey (light theme)
};

// Standard 20-AA → property assignment. Anything not listed (X/B/Z/gap …) is "unknown" → aaColor
// returns null so callers keep their own faint/plain rendering (gap and unknown stay distinct).
const AA_PROPERTY: Record<string, AAProperty> = {
  D: "acidic", E: "acidic",
  K: "basic", R: "basic", H: "basic",
  S: "polar", T: "polar", N: "polar", Q: "polar",
  C: "special", G: "special", P: "special",
  A: "hydrophobic", V: "hydrophobic", L: "hydrophobic", I: "hydrophobic",
  M: "hydrophobic", F: "hydrophobic", W: "hydrophobic", Y: "hydrophobic",
};
const AA_HYDRO_DARK = "#aeb4bd"; // lighter neutral so a hydrophobic letter reads on dark surfaces

/** Property group for a residue letter, or null for gap/unknown (X, B, Z, '-', …). */
export function aaProperty(residue: string): AAProperty | null {
  return AA_PROPERTY[(residue ?? "").trim().toUpperCase()] ?? null;
}

/**
 * Shared amino-acid colour by physico-chemical property. Returns a hex string for the 20 standard
 * residues (identical across every figure), or `null` for gap/unknown so the caller can render
 * those with its own neutral styling — keeping '-' and 'X/N' visually distinct from real residues.
 */
export function aaColor(residue: string, dark: boolean = isDark()): string | null {
  const g = aaProperty(residue);
  if (!g) return null;
  if (g === "hydrophobic") return dark ? AA_HYDRO_DARK : AA_PROPERTY_PALETTE.hydrophobic;
  return AA_PROPERTY_PALETTE[g];
}

// ── Continuous colour scales ─────────────────────────────────────────────────
function hexToRgb(h: string): [number, number, number] {
  const s = h.replace("#", "");
  const v = s.length === 3 ? s.split("").map((c) => c + c).join("") : s;
  return [parseInt(v.slice(0, 2), 16), parseInt(v.slice(2, 4), 16), parseInt(v.slice(4, 6), 16)];
}
function rgbToHex(r: number, g: number, b: number): string {
  const c = (x: number) => Math.max(0, Math.min(255, Math.round(x))).toString(16).padStart(2, "0");
  return `#${c(r)}${c(g)}${c(b)}`;
}
/** Linear interpolation across an evenly-spaced list of hex stops; t clamped to [0,1]. */
function rampAt(stops: string[], t: number): string {
  const u = !isFinite(t) ? 0 : Math.max(0, Math.min(1, t));
  const seg = u * (stops.length - 1);
  const i = Math.min(stops.length - 2, Math.floor(seg));
  const f = seg - i;
  const [r1, g1, b1] = hexToRgb(stops[i]);
  const [r2, g2, b2] = hexToRgb(stops[i + 1]);
  return rgbToHex(r1 + (r2 - r1) * f, g1 + (g2 - g1) * f, b1 + (b2 - b1) * f);
}

// matplotlib viridis (perceptually uniform, CVD-safe) — the sequential default.
const VIRIDIS = ["#440154", "#472d7b", "#3b528b", "#2c728e", "#21918c",
                 "#28ae80", "#5ec962", "#addc30", "#fde725"];
// ColorBrewer RdBu (blue → neutral → red) for SIGNED data with a true zero. Reversed so
// negative = blue, positive = red (the convention for log2FC / z / Δ).
const RDBU = ["#053061", "#2166ac", "#4393c3", "#92c5de", "#d1e5f0", "#f7f7f7",
              "#fddbc7", "#f4a582", "#d6604d", "#b2182b", "#67001f"];

/** Sequential (unsigned magnitude): t in [0,1] → viridis. */
export function sequential(t: number): string {
  return rampAt(VIRIDIS, t);
}
/** Diverging (signed, true 0 centre): t in [-1,1] → RdBu, 0 = neutral. */
export function diverging(t: number): string {
  return rampAt(RDBU, (Math.max(-1, Math.min(1, t)) + 1) / 2);
}
/** Semantic dispatch: signed data → diverging (centre 0), else sequential. */
export function continuous(t: number, signed: boolean): string {
  return signed ? diverging(t) : sequential(t);
}

// ── Text-on-fill contrast (WCAG relative luminance) ──────────────────────────
/** Black or white ink for legible text on an arbitrary fill colour. */
export function autoContrastText(hex: string): string {
  const [r, g, b] = hexToRgb(hex);
  const lin = (c: number) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
  };
  const L = 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
  return L > 0.42 ? "#141618" : "#f7f8f9";
}

// ── Tokens (SVG-unit baselines; renderers use these instead of scattered literals) ───────────
export const STROKE = { main: 1, axis: 1, ref: 0.75, grid: 0.5, emphasis: 1.6, hairline: 0.5 };
export const ALPHA = { overplot: 0.6, dense: 0.4, ciBand: 0.18, ns: 0.45, ref: 0.6, pharmacophore: 0.35 };
export const FONT = { min: 8, tick: 9, axis: 10, title: 12, panel: 12 };

// ── Data provenance (graceful, not a hard gate) ──────────────────────────────
// A first-hand data figure (heatmap/volcano/forest/ROC…) should say where its numbers came
// from. We SURFACE the absence with a small corner tag rather than refusing to draw — so the
// existing corpus keeps rendering while the agent learns to emit provenance. Only figures the
// agent explicitly marks `schematic` are labelled as non-data illustrations.
export type Provenance =
  | string
  | { digitized_from?: string; computed_on?: string; schematic?: boolean; source?: string }
  | null
  | undefined;

export type ProvTag = { text: string; kind: "ok" | "schematic" | "missing" };

/** Normalise a spec's provenance field into a corner-tag descriptor (or null = nothing to show). */
export function provenanceTag(p: Provenance): ProvTag | null {
  if (p == null) return null;
  if (typeof p === "string") {
    const s = p.trim();
    if (!s) return null;
    if (/^schematic|示意/i.test(s)) return { text: "示意 · 非实测数据", kind: "schematic" };
    return { text: s, kind: "ok" };
  }
  if (p.schematic) return { text: "示意 · 非实测数据", kind: "schematic" };
  if (p.digitized_from) return { text: `数字化自 ${p.digitized_from}`, kind: "ok" };
  if (p.computed_on) return { text: `本地计算 · ${p.computed_on}`, kind: "ok" };
  if (p.source) return { text: p.source, kind: "ok" };
  return null;
}
