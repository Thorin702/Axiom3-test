// Shared cleanup + inline-markdown rendering for the reviewer's critique.
//
// The reviewer model (DeepSeek/GLM) emits markdown — bullets, `1.`/`①` numbering, `**bold**`,
// `` `code` ``, headers. Rendering that as raw text leaks "乱七八糟的符号" into the UI. So we (a)
// strip leading list/heading/number markers when splitting into findings, and (b) render each
// finding as SANITIZED inline markdown, so emphasis/code render as HTML instead of literal symbols.
import { marked } from "marked";
import DOMPurify from "dompurify";

// Split a bullet/line-based critique into discrete findings, stripping list markers, numbering and
// headers so no stray markdown prefix survives. Multi-marker prefixes (e.g. "1. **") are peeled once.
export function toFindings(review: string): string[] {
  return review
    .trim()
    .split("\n")
    .map((l) =>
      l
        .replace(/^\s*#{1,6}\s*/, "") // md headers  ## foo
        .replace(/^\s*>\s?/, "") // blockquote
        .replace(/^\s*[-*•·]\s?/, "") // bullets
        .replace(/^\s*[（(]?\s*(?:\d{1,2}|[①②③④⑤⑥⑦⑧⑨⑩⑪⑫])\s*[)）.、．:：]\s*/, "") // 1. / 1) /（1）/ ①、
        .trim()
    )
    .filter((l) => l.length > 1);
}

const ALL_CLEAR = /^(no (issues|problems|concerns|critique)|the outputs are solid|looks (good|solid)|nothing to flag|整体扎实|未见明显问题)/i;

// True when the critique is a single "all good" line (→ render as a clean checkmark, not a finding).
export function isAllClear(review: string, findings: string[]): boolean {
  return findings.length <= 1 && ALL_CLEAR.test(review.trim());
}

const INLINE_TAGS = ["strong", "em", "b", "i", "code", "del", "s", "mark", "sub", "sup", "span", "br"];

// Render one finding as inline markdown (bold/code/emphasis only — no block elements), sanitized.
export function renderInlineMd(s: string): string {
  const raw = marked.parseInline(s, { async: false }) as string;
  return DOMPurify.sanitize(raw, { ALLOWED_TAGS: INLINE_TAGS, ALLOWED_ATTR: ["class"], ALLOW_DATA_ATTR: false });
}
