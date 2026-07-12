// A figure the analysis agent COMPUTED from real data in the persistent Python kernel —
// a matplotlib PNG returned as base64. Trusted (we produced it in our own kernel), so it
// renders as an inline image. This is what makes a report's figures computed, not asserted.
//
// Body (```pyfig): a raw base64 PNG string (no data: prefix).
import { useEffect, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { Download, Maximize2, X } from "lucide-react";

export function Pyfig({ spec, caption }: { spec: string; caption?: string }) {
  const b64 = spec.trim().replace(/\s+/g, "");
  const [zoom, setZoom] = useState(false);
  // Escape closes the lightbox.
  useEffect(() => {
    if (!zoom) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") setZoom(false); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [zoom]);
  if (!b64) return <div className="ax-fig__err">空图 · empty figure</div>;
  const src = `data:image/png;base64,${b64}`;
  const name = ((caption || "figure").slice(0, 48).replace(/[^\p{L}\p{N} _-]/gu, "").trim() || "figure") + ".png";

  // Download via the OS save dialog + a native write command. A `data:` <a download> is
  // silently dropped by WKWebView in the packaged app (no WKDownloadDelegate), so we decode
  // the base64 here and hand the raw bytes to Rust — the same path exportReport uses.
  async function download() {
    try {
      const path = await save({ defaultPath: name, filters: [{ name: "PNG", extensions: ["png"] }] });
      if (typeof path !== "string") return;
      const bin = atob(b64);
      const bytes = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
      await invoke("save_png", { path, bytes: Array.from(bytes) });
    } catch {
      /* cancelled or unavailable */
    }
  }

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame">
        <img
          className="ax-fig__pyfig"
          src={src}
          alt={caption || "computed figure"}
          onClick={() => setZoom(true)}
          style={{ maxWidth: "100%", height: "auto", display: "block", borderRadius: 8, background: "#fff", cursor: "zoom-in" }}
        />
        <div className="ax-fig__tools">
          <button className="ax-fig__tool" onClick={(e) => { e.stopPropagation(); download(); }} title="下载 PNG · Download" aria-label="download">
            <Download size={14} aria-hidden />
          </button>
          <button className="ax-fig__tool" onClick={(e) => { e.stopPropagation(); setZoom(true); }} title="放大 · Enlarge" aria-label="enlarge">
            <Maximize2 size={14} aria-hidden />
          </button>
        </div>
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">计算图 · computed</span>
        {caption || ""}
      </figcaption>

      {zoom && (
        <div className="ax-lightbox" onClick={() => setZoom(false)} role="dialog" aria-modal="true">
          <button className="ax-lightbox__close" onClick={() => setZoom(false)} title="关闭 · Close (Esc)" aria-label="close">
            <X size={20} aria-hidden />
          </button>
          <img className="ax-lightbox__img" src={src} alt={caption || "figure"} onClick={(e) => e.stopPropagation()} />
          <button className="ax-lightbox__dl" onClick={(e) => { e.stopPropagation(); download(); }}>
            <Download size={15} aria-hidden /> 下载 PNG
          </button>
        </div>
      )}
    </figure>
  );
}
