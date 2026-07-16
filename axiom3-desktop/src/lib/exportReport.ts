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
/* print / PDF: A4 pages, keep figures & tables whole, headings with their content */
@page{size:A4;margin:1.7cm 1.5cm;}
@media print{
  html,body{background:#fff;}
  .ax-report{max-width:none;margin:0;padding:0;}
  figure,table,.ax-fig,.ax-fig__frame,.tablewrap,pre{page-break-inside:avoid;break-inside:avoid;}
  h1,h2,h3{page-break-after:avoid;break-after:avoid;}
  img,svg{max-width:100%!important;}
  .ax-fig__tools,.ax-lightbox{display:none!important;}
}
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

/** Open the rendered report in a hidden iframe and trigger the print dialog — the user chooses
 *  "Save as PDF". Reuses the exact rendered figures/tables + the @page/print CSS above. */
export function exportPdf(artifactNode: HTMLElement, title: string, theme: string): void {
  const html = buildReportHtml(artifactNode, title, theme);
  const iframe = document.createElement("iframe");
  iframe.setAttribute("aria-hidden", "true");
  Object.assign(iframe.style, { position: "fixed", right: "0", bottom: "0", width: "0", height: "0", border: "0", opacity: "0" });
  document.body.appendChild(iframe);
  const doc = iframe.contentDocument || iframe.contentWindow?.document;
  if (!doc) { iframe.remove(); return; }
  const fire = () =>
    setTimeout(() => {
      try {
        iframe.contentWindow?.focus();
        iframe.contentWindow?.print();
      } finally {
        setTimeout(() => iframe.remove(), 1500);
      }
    }, 400); // let embedded images/fonts settle
  iframe.onload = fire;
  doc.open();
  doc.write(html);
  doc.close();
  if (doc.readyState === "complete") fire();
}

/** Build a Word-openable (.doc) document: the report body wrapped in the Office/Word MSO header so
 *  Word renders the styles + embedded PNG figures. (Inline-SVG figures need Word 2016+.) */
export function buildWordHtml(artifactNode: HTMLElement, title: string, theme: string): string {
  const inner = buildReportHtml(artifactNode, title, theme);
  const style = inner.match(/<style>([\s\S]*?)<\/style>/)?.[1] ?? "";
  const body = inner.match(/<body>([\s\S]*?)<\/body>/)?.[1] ?? inner;
  const t = esc(title || "研究报告");
  return `<html xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:w="urn:schemas-microsoft-com:office:word" xmlns="http://www.w3.org/TR/REC-html40">
<head><meta charset="utf-8"><title>${t}</title>
<!--[if gte mso 9]><xml><w:WordDocument><w:View>Print</w:View><w:Zoom>100</w:Zoom></w:WordDocument></xml><![endif]-->
<style>@page{size:A4;margin:2cm;}
${style}</style>
</head><body>${body}</body></html>`;
}
