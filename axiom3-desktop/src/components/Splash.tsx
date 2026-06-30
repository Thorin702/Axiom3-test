// Boot splash — a brief branded loading screen shown while the app mounts, then fades
// out. Uses the design tokens, so it tracks the active theme/palette. The accent
// "scanline" evokes the product's checking-as-it-goes identity.
import { useEffect, useState } from "react";
import type { Lang } from "../i18n";

export function Splash({ lang, onDone }: { lang: Lang; onDone: () => void }) {
  const [leaving, setLeaving] = useState(false);
  useEffect(() => {
    const t1 = setTimeout(() => setLeaving(true), 950);
    const t2 = setTimeout(onDone, 1350);
    return () => {
      clearTimeout(t1);
      clearTimeout(t2);
    };
  }, [onDone]);

  return (
    <div className={`ax-splash ${leaving ? "is-leaving" : ""}`} aria-hidden>
      <div className="ax-splash__mark">
        <span className="ax-splash__word">Axiom3</span>
        <span className="ax-splash__scan">
          <span />
        </span>
      </div>
      <p className="ax-splash__tag">
        {lang === "zh" ? "校验为中心的多智能体工作台" : "a verification-first multi-agent workbench"}
      </p>
    </div>
  );
}
