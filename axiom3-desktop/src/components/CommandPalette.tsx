// ⌘K command palette — a global launcher overlay. Presentational + keyboard-driven; it
// holds no app knowledge: App passes a flat item list (nav destinations + slash commands)
// and a pick handler. Esc / click-outside closes; ↑↓ move; ↵ runs the highlighted item.
import { useEffect, useRef, useState } from "react";
import { CornerDownLeft } from "lucide-react";
import type { Lang } from "../i18n";

export type PaletteItem = { key: string; label: string; sub?: string; kind: "nav" | "cmd"; disabled?: boolean };

export function CommandPalette({
  lang,
  items,
  onPick,
  onClose,
}: {
  lang: Lang;
  items: PaletteItem[];
  onPick: (it: PaletteItem) => void;
  onClose: () => void;
}) {
  const [q, setQ] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const query = q.trim().toLowerCase();
  const filtered = items.filter(
    (i) => query === "" || i.label.toLowerCase().includes(query) || (i.sub ?? "").toLowerCase().includes(query)
  );
  const idx = Math.min(active, Math.max(0, filtered.length - 1));

  function onKey(e: React.KeyboardEvent) {
    if (e.key === "Escape") onClose();
    else if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((a) => Math.min(a + 1, filtered.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => Math.max(a - 1, 0));
    } else if (e.key === "Enter") {
      const it = filtered[idx];
      if (it && !it.disabled) onPick(it);
    }
  }

  return (
    <div className="ax-pal-overlay" onClick={onClose}>
      <div className="ax-pal" onClick={(e) => e.stopPropagation()}>
        <input
          ref={inputRef}
          className="ax-pal__input"
          value={q}
          onChange={(e) => {
            setQ(e.currentTarget.value);
            setActive(0);
          }}
          onKeyDown={onKey}
          placeholder={lang === "zh" ? "跳转或运行任务…   输入 / 查看任务类型" : "Jump to, or run a task…   type / for task types"}
        />
        <div className="ax-pal__list">
          {filtered.length === 0 ? (
            <div className="ax-pal__item" style={{ color: "var(--ax-ink-faint)", cursor: "default" }}>
              {lang === "zh" ? "没有匹配项" : "No matches"}
            </div>
          ) : (
            filtered.map((it, i) => (
              <button
                key={it.kind + it.key}
                className={`ax-pal__item ${i === idx ? "is-active" : ""}`}
                disabled={it.disabled}
                style={it.disabled ? { opacity: 0.5 } : undefined}
                onMouseEnter={() => setActive(i)}
                onClick={() => onPick(it)}
              >
                <span className="ax-badge" style={{ minWidth: 38, justifyContent: "center" }}>
                  {it.kind === "cmd" ? "/" : (lang === "zh" ? "前往" : "Go")}
                </span>
                <span className="ax-row__title" style={{ flex: "0 1 auto" }}>{it.label}</span>
                {it.sub && <span className="ax-row__sub">{it.sub}</span>}
                {i === idx && !it.disabled && <CornerDownLeft size={13} aria-hidden style={{ marginLeft: "auto", opacity: 0.5 }} />}
              </button>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
