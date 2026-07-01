// Unified top banner (feature-list #8) — one place for degradation/offline/error instead
// of inline errors scattered around. Offline is detected live; the error is dismissible.
import { useEffect, useState } from "react";
import { X, WifiOff, AlertTriangle } from "lucide-react";
import type { Lang } from "../i18n";

export function AppBanner({ lang, error, onDismiss }: { lang: Lang; error: string; onDismiss: () => void }) {
  const zh = lang === "zh";
  const [online, setOnline] = useState(typeof navigator !== "undefined" ? navigator.onLine : true);

  useEffect(() => {
    const on = () => setOnline(true);
    const off = () => setOnline(false);
    window.addEventListener("online", on);
    window.addEventListener("offline", off);
    return () => {
      window.removeEventListener("online", on);
      window.removeEventListener("offline", off);
    };
  }, []);

  if (online && !error) return null;

  return (
    <div className="ax-banners">
      {!online && (
        <div className="ax-banner ax-banner--warn">
          <WifiOff size={14} aria-hidden />
          <span className="ax-banner__msg">
            {zh
              ? "离线 — 跨模型核查与文献检索暂停,本地确定性检查仍可用。"
              : "Offline — cross-model checks + literature retrieval are paused; local deterministic checks still work."}
          </span>
        </div>
      )}
      {error && (
        <div className="ax-banner ax-banner--err">
          <AlertTriangle size={14} aria-hidden />
          <span className="ax-banner__msg">{error}</span>
          <button className="ax-iconbtn" onClick={onDismiss} title={zh ? "关闭" : "Dismiss"} aria-label={zh ? "关闭" : "Dismiss"}>
            <X size={13} aria-hidden />
          </button>
        </div>
      )}
    </div>
  );
}
