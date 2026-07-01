// Persistent bottom status bar (feature-list #23) — live, backend-wired lamps: hash-chain
// integrity, cross-vendor independence (≥2 connected families), connected providers, and
// the open project's event count. Lamps are neutral signals, never truth verdicts.
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Lang } from "../i18n";
import type { ProjectStatus } from "../lib/types";

export function StatusBar({ lang, status }: { lang: Lang; status: ProjectStatus | null }) {
  const zh = lang === "zh";
  const [providers, setProviders] = useState(0);

  useEffect(() => {
    const load = () =>
      invoke<{ family: string; connected: boolean }[]>("provider_status")
        .then((ps) => setProviders(ps.filter((p) => p.connected).length))
        .catch(() => {});
    load();
    const t = setInterval(load, 5000);
    return () => clearInterval(t);
  }, []);

  return (
    <footer className="ax-statusbar">
      {status ? (
        <span className="ax-sb__item" title={status.chain_head ?? undefined}>
          <span className={`ax-lamp ${status.chain_ok ? "is-green" : "is-amber"}`} />
          {zh ? "哈希链" : "Chain"} {status.chain_ok ? (zh ? "完整" : "intact") : zh ? "待核" : "review"}
        </span>
      ) : (
        <span className="ax-sb__item">
          <span className="ax-lamp is-neutral" />
          {zh ? "无项目" : "No project"}
        </span>
      )}

      <span className="ax-sb__item" title={zh ? "跨厂商独立性:至少 2 个独立模型家族" : "cross-vendor independence: ≥2 model families"}>
        <span className={`ax-lamp ${providers >= 2 ? "is-green" : "is-amber"}`} />
        {zh ? "独立性" : "Independence"} {Math.min(providers, 2)}/2
      </span>

      <span className="ax-sb__item">
        <span className={`ax-lamp ${providers > 0 ? "is-green" : "is-neutral"}`} />
        {providers} {zh ? "个模型已连接" : "connected"}
      </span>

      {status && (
        <span className="ax-sb__item ax-sb__right">
          {status.event_count} {zh ? "个事件" : "events"}
        </span>
      )}
    </footer>
  );
}
