// A figure the analysis agent COMPUTED from real data in the persistent Python kernel —
// a matplotlib PNG returned as base64. Trusted (we produced it in our own kernel), so it
// renders as an inline image. This is what makes a report's figures computed, not asserted.
//
// Body (```pyfig): a raw base64 PNG string (no data: prefix).
export function Pyfig({ spec, caption }: { spec: string; caption?: string }) {
  const b64 = spec.trim().replace(/\s+/g, "");
  if (!b64) return <div className="ax-fig__err">空图 · empty figure</div>;
  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame">
        <img
          className="ax-fig__pyfig"
          src={`data:image/png;base64,${b64}`}
          alt={caption || "computed figure"}
          style={{ maxWidth: "100%", height: "auto", display: "block", borderRadius: 8, background: "#fff" }}
        />
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">计算图 · computed</span>
        {caption || ""}
      </figcaption>
    </figure>
  );
}
