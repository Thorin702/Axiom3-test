// Boot splash — a brief branded loading screen shown while the app mounts, then fades
// out. Uses the design tokens, so it tracks the active theme/palette. The accent
// "scanline" evokes the product's checking-as-it-goes identity.
import { useEffect, useState } from "react";
import type { Lang } from "../i18n";

// Staged boot progress (feature-list #5): keychain → encrypted store → session → models.
const STAGES = {
  zh: ["解锁钥匙串", "打开加密库", "恢复会话", "探测模型", "就绪"],
  en: ["Unlocking keychain", "Opening store", "Restoring session", "Probing models", "Ready"],
};

export function Splash({ lang, onDone }: { lang: Lang; onDone: () => void }) {
  const [leaving, setLeaving] = useState(false);
  const [stage, setStage] = useState(0);
  const stages = STAGES[lang];

  useEffect(() => {
    const iv = setInterval(() => setStage((s) => Math.min(s + 1, stages.length - 1)), 280);
    const t1 = setTimeout(() => setLeaving(true), 1500);
    const t2 = setTimeout(onDone, 1900);
    return () => {
      clearInterval(iv);
      clearTimeout(t1);
      clearTimeout(t2);
    };
  }, [onDone, stages.length]);

  return (
    <div className={`ax-splash ${leaving ? "is-leaving" : ""}`} aria-hidden>
      <div className="ax-splash__mark">
        <span className="ax-splash__word">Axiom3</span>
        <span className="ax-splash__scan">
          <span />
        </span>
      </div>
      <div className="ax-splash__stages">
        {stages.map((label, i) => (
          <span key={label} className={`ax-splash__stage ${i < stage ? "is-done" : i === stage ? "is-now" : ""}`}>
            <span className="ax-splash__dot" />
            {label}
          </span>
        ))}
      </div>
    </div>
  );
}
