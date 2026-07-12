// A thin vertical drag bar for resizing a panel. Positioned absolutely over a column boundary
// by the caller (via style). On drag it reports the pointer's clientX; the caller maps that to
// a new panel width (with its own clamp) and re-lays the grid. Double-click resets via onReset.
import { useEffect, useRef } from "react";

export function ResizeHandle({
  onResize,
  onReset,
  style,
  className,
}: {
  onResize: (clientX: number) => void;
  onReset?: () => void;
  style?: React.CSSProperties;
  className?: string;
}) {
  const active = useRef(false);
  // Keep the latest onResize without re-subscribing the window listeners every render.
  const cb = useRef(onResize);
  cb.current = onResize;
  useEffect(() => {
    function move(e: MouseEvent) {
      if (!active.current) return;
      e.preventDefault();
      cb.current(e.clientX);
    }
    function up() {
      if (!active.current) return;
      active.current = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    }
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    window.addEventListener("blur", up); // safety: release if the mouse-up lands outside the window
    return () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      window.removeEventListener("blur", up);
    };
  }, []);
  return (
    <div
      className={`ax-resize ${className || ""}`}
      style={style}
      role="separator"
      aria-orientation="vertical"
      onMouseDown={(e) => {
        e.preventDefault();
        active.current = true;
        document.body.style.cursor = "col-resize";
        document.body.style.userSelect = "none";
      }}
      onDoubleClick={onReset}
    />
  );
}
