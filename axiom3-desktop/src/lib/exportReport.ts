// Compile a rendered artifact into a SELF-CONTAINED html report the user can open/share
// offline. Everything is inlined: the native SVG figures stay as-is, any <canvas> (Chart.js)
// is snapshotted to a PNG <img>, and all of the app's CSS (theme + KaTeX) is embedded so the
// figures render without the app. WebGL canvases (3D protein) may snapshot blank if the
// renderer didn't preserve its drawing buffer — an acceptable edge for a static export.

function esc(s: string): string {
  return s.replace(/[<>&]/g, (c) => ({ "<": "&lt;", ">": "&gt;", "&": "&amp;" }[c] as string));
}

/** Serialize every same-origin stylesheet rule into one CSS string. */
function collectCss(): string {
  let css = "";
  for (const sheet of Array.from(document.styleSheets)) {
    try {
      for (const rule of Array.from(sheet.cssRules)) css += rule.cssText + "\n";
    } catch {
      /* cross-origin sheet — not readable, skip */
    }
  }
  return css;
}

/** Build the standalone report HTML from a rendered artifact node. */
export function buildReportHtml(artifactNode: HTMLElement, title: string, theme: string): string {
  const clone = artifactNode.cloneNode(true) as HTMLElement;
  // Snapshot live canvases (Chart.js / WebGL) into <img> so they survive in a static file.
  const live = artifactNode.querySelectorAll("canvas");
  const cloned = clone.querySelectorAll("canvas");
  cloned.forEach((c, i) => {
    const src = live[i] as HTMLCanvasElement | undefined;
    if (!src) return;
    try {
      const img = document.createElement("img");
      img.src = src.toDataURL("image/png");
      img.style.maxWidth = "100%";
      img.style.height = "auto";
      c.replaceWith(img);
    } catch {
      /* tainted / WebGL-without-preserveDrawingBuffer — leave the empty canvas */
    }
  });

  const t = esc(title || "研究报告");
  const css = collectCss();
  return `<!doctype html>
<html lang="zh" data-theme="${theme === "dark" ? "dark" : "light"}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${t}</title>
<style>
${css}
html,body{background:var(--ax-paper,#faf9f5);color:var(--ax-ink,#292827);margin:0;}
.ax-report{max-width:840px;margin:0 auto;padding:40px 28px 80px;font-family:var(--ax-sans);}
.ax-report__h{font-family:var(--ax-serif);font-size:25px;line-height:1.25;margin:0 0 8px;color:var(--ax-ink);}
.ax-report__meta{color:var(--ax-ink-faint);font-size:12px;border-bottom:0.5px solid var(--ax-line);padding-bottom:18px;margin-bottom:26px;}
</style>
</head>
<body>
<div class="ax-report">
<h1 class="ax-report__h">${t}</h1>
<div class="ax-report__meta">Axiom3 · 图与表由真实数据计算得到,可独立复核 · 本文件自包含,可离线打开</div>
${clone.outerHTML}
</div>
</body>
</html>`;
}
