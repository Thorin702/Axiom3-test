// Human labels for the orchestration phases the run_event stream reports. Shared by the
// in-thread thinking chain (ThreadActivity) and the right activity rail so the two never
// drift. Keys are the raw `phase` strings emitted by the engine (coding / workflow / search).
export const PHASE_LABELS: Record<string, { zh: string; en: string }> = {
  planning: { zh: "规划", en: "Planning" },
  executing: { zh: "执行", en: "Executing" },
  searching: { zh: "自适应搜索", en: "Adaptive search" },
  verifying: { zh: "审查", en: "Reviewing" },
  synthesizing: { zh: "汇总", en: "Synthesizing" },
  computing: { zh: "计算图", en: "Computing figures" },
  done: { zh: "完成", en: "Done" },
};

export function phaseLabel(p: string, zh: boolean): string {
  const l = PHASE_LABELS[p];
  return l ? (zh ? l.zh : l.en) : p;
}
