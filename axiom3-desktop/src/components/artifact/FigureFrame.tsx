// Wraps a NATIVE figure renderer (the SVG/canvas ones: mermaid/chart/heatmap/tree/network/…) to
// give it the SAME click-to-lightbox zoom + download UX that Pyfig (computed PNGs) already has.
// The figure renders ONCE in the thread; on enlarge/download we rasterize its <svg>/<canvas> to a
// PNG and reuse Pyfig's exact image lightbox (wheel-zoom, drag-pan, Esc, native save).
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { Download, Maximize2, X } from "lucide-react";

/** Rasterize the first <svg> (or <canvas>) inside `el` to a PNG blob at 2× for crispness. */
async function rasterize(el: HTMLElement): Promise<Blob | null> {
  const svg = el.querySelector("svg");
  if (svg) {
    const vb = svg.viewBox?.baseVal;
    const w = Math.round((vb && vb.width) || svg.getBoundingClientRect().width || 900);
    const h = Math.round((vb && vb.height) || svg.getBoundingClientRect().height || 600);
    const clone = svg.cloneNode(true) as SVGSVGElement;
    clone.setAttribute("width", String(w));
    clone.setAttribute("height", String(h));
    if (!clone.getAttribute("xmlns")) clone.setAttribute("xmlns", "http://www.w3.org/2000/svg");
    const str = new XMLSerializer().serializeToString(clone);
    const url = URL.createObjectURL(new Blob([str], { type: "image/svg+xml;charset=utf-8" }));
    try {
      const img = new Image();
      await new Promise((res, rej) => { img.onload = () => res(null); img.onerror = rej; img.src = url; });
      const s = 2;
      const canvas = document.createElement("canvas");
      canvas.width = w * s; canvas.height = h * s;
      const ctx = canvas.getContext("2d");
      if (!ctx) return null;
      ctx.fillStyle = "#fff"; ctx.fillRect(0, 0, canvas.width, canvas.height);
      ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
      return await new Promise<Blob | null>((res) => canvas.toBlob(res, "image/png"));
    } finally {
      URL.revokeObjectURL(url);
    }
  }
  const canvas = el.querySelector("canvas");
  if (canvas) return await new Promise<Blob | null>((res) => canvas.toBlob(res, "image/png"));
  return null;
}

export function FigureFrame({ children, kind }: { children: React.ReactNode; kind?: string }) {
  const [zoom, setZoom] = useState(false);
  const [url, setUrl] = useState<string | null>(null); // rasterized PNG object-URL for the lightbox
  const [scale, setScale] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const frameRef = useRef<HTMLDivElement>(null);
  const imgRef = useRef<HTMLImageElement>(null);
  const dragFrom = useRef<{ x: number; y: number } | null>(null);

  useLayoutEffect(() => { if (zoom) { setScale(1); setPan({ x: 0, y: 0 }); } }, [zoom]);

  // Wheel-zoom + drag-pan + Esc (mirrors Pyfig; native non-passive wheel so preventDefault works).
  useEffect(() => {
    if (!zoom) return;
    const el = imgRef.current;
    const onWheel = (e: WheelEvent) => { e.preventDefault(); setScale((s) => Math.min(8, Math.max(0.4, s * (e.deltaY < 0 ? 1.15 : 0.87)))); };
    const onMove = (e: MouseEvent) => { if (dragFrom.current) setPan({ x: e.clientX - dragFrom.current.x, y: e.clientY - dragFrom.current.y }); };
    const onUp = () => { dragFrom.current = null; };
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") setZoom(false); };
    el?.addEventListener("wheel", onWheel, { passive: false });
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    window.addEventListener("keydown", onKey);
    return () => {
      el?.removeEventListener("wheel", onWheel);
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      window.removeEventListener("keydown", onKey);
    };
  }, [zoom]);

  // Release the object-URL when the lightbox closes.
  useEffect(() => () => { if (url) URL.revokeObjectURL(url); }, [url]);

  async function openZoom() {
    if (!frameRef.current) return;
    const blob = await rasterize(frameRef.current);
    if (!blob) return; // nothing rasterizable → leave the inline figure as-is
    setUrl((old) => { if (old) URL.revokeObjectURL(old); return URL.createObjectURL(blob); });
    setZoom(true);
  }

  async function download() {
    if (!frameRef.current) return;
    try {
      const blob = await rasterize(frameRef.current);
      if (!blob) return;
      const path = await save({ defaultPath: (kind || "figure") + ".png", filters: [{ name: "PNG", extensions: ["png"] }] });
      if (typeof path !== "string") return;
      const bytes = new Uint8Array(await blob.arrayBuffer());
      await invoke("save_png", { path, bytes: Array.from(bytes) });
    } catch { /* cancelled or unavailable */ }
  }

  const canPan = scale > 1;
  return (
    // A borderless wrapper (the child renderer already draws its own .ax-fig frame + caption).
    // We only add the hover tools + the shared lightbox on top — no nested borders.
    <div className="ax-figwrap" ref={frameRef}>
      <div className="ax-figwrap__click" onClick={openZoom} style={{ cursor: "zoom-in" }}>{children}</div>
      <div className="ax-figwrap__tools">
        <button className="ax-fig__tool" onClick={(e) => { e.stopPropagation(); download(); }} title="下载 PNG · Download" aria-label="download">
          <Download size={14} aria-hidden />
        </button>
        <button className="ax-fig__tool" onClick={(e) => { e.stopPropagation(); openZoom(); }} title="放大 · Enlarge" aria-label="enlarge">
          <Maximize2 size={14} aria-hidden />
        </button>
      </div>

      {zoom && url && (
        <div className="ax-lightbox" onClick={() => setZoom(false)} role="dialog" aria-modal="true">
          <button className="ax-lightbox__close" onClick={() => setZoom(false)} title="关闭 · Close (Esc)" aria-label="close">
            <X size={20} aria-hidden />
          </button>
          <img
            ref={imgRef}
            className="ax-lightbox__img"
            src={url}
            alt="figure"
            onClick={(e) => e.stopPropagation()}
            onMouseDown={(e) => { if (!canPan) return; e.preventDefault(); dragFrom.current = { x: e.clientX - pan.x, y: e.clientY - pan.y }; }}
            style={{
              transform: `translate(${pan.x}px, ${pan.y}px) scale(${scale})`,
              transition: dragFrom.current ? "none" : "transform .05s linear",
              cursor: canPan ? "grab" : "default",
            }}
          />
          <span className="ax-lightbox__hint">滚轮缩放 · 拖拽平移 · Esc 关闭</span>
          <button className="ax-lightbox__dl" onClick={(e) => { e.stopPropagation(); download(); }}>
            <Download size={15} aria-hidden /> 下载 PNG
          </button>
        </div>
      )}
    </div>
  );
}
