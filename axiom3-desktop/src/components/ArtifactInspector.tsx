// Rigor attached to the ARTIFACT, not parked in separate destinations. Claude Science folds
// the reviewer + provenance into the thing you made; so do we. These tabs are scoped to the
// session that produced this artifact (evidence + neutralized injections come from it), while
// the chain views are workspace-level facts about the record it was written into.
//
// RED LINE: none of these tabs judge whether the content is TRUE. They show what was checked,
// what evidence was retrieved, what was neutralized, and where the artifact sits in the
// tamper-evident record.
import { useState } from "react";
import { Quote, ShieldCheck, GitBranch, FileCheck } from "lucide-react";
import type { Lang } from "../i18n";
import type { Task } from "../lib/types";
import { EvidenceView } from "./EvidenceView";
import { ShieldView } from "./ShieldView";
import { ProvenanceView } from "./ProvenanceView";
import { IntegrityView } from "./IntegrityView";

type Tab = "evidence" | "shield" | "provenance" | "integrity";

const TABS: { key: Tab; zh: string; en: string; icon: typeof Quote }[] = [
  { key: "evidence", zh: "证据", en: "Evidence", icon: Quote },
  { key: "shield", zh: "护盾", en: "Shield", icon: ShieldCheck },
  { key: "provenance", zh: "溯源", en: "Provenance", icon: GitBranch },
  { key: "integrity", zh: "完整性", en: "Integrity", icon: FileCheck },
];

export function ArtifactInspector({ lang, task }: { lang: Lang; task: Task }) {
  const zh = lang === "zh";
  const [open, setOpen] = useState(false);
  const [tab, setTab] = useState<Tab>("evidence");

  return (
    <section className="ax-inspect">
      <button className="ax-inspect__toggle" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <ShieldCheck size={13} aria-hidden />
        {zh ? "检查与证据 · 附于此产物" : "Checks & evidence · attached to this artifact"}
        <span className="ax-inspect__chev">{open ? "−" : "+"}</span>
      </button>

      {open && (
        <>
          <div className="ax-inspect__tabs" role="tablist">
            {TABS.map((t) => {
              const Icon = t.icon;
              return (
                <button
                  key={t.key}
                  role="tab"
                  aria-selected={tab === t.key}
                  className={`ax-inspect__tab ${tab === t.key ? "is-on" : ""}`}
                  onClick={() => setTab(t.key)}
                >
                  <Icon size={12} aria-hidden /> {zh ? t.zh : t.en}
                </button>
              );
            })}
          </div>
          <div className="ax-inspect__body">
            {/* scoped to THIS session's artifact */}
            {tab === "evidence" && <EvidenceView lang={lang} tasks={[task]} />}
            {tab === "shield" && <ShieldView lang={lang} tasks={[task]} />}
            {/* workspace-level facts about the record this artifact was written into */}
            {tab === "provenance" && <ProvenanceView lang={lang} />}
            {tab === "integrity" && <IntegrityView lang={lang} />}
          </div>
        </>
      )}
    </section>
  );
}
