import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

// Tag the OS so CSS can adapt to Windows/Chromium quirks the Retina-Mac look didn't hit — 0.5px
// hairline borders that round to 0px, and the chunky default scrollbar (see `[data-os="windows"]`
// in theme.css). WebView2's UA carries "Windows NT"; WKWebView on macOS carries "Macintosh".
if (/Windows/i.test(navigator.userAgent)) {
  document.documentElement.dataset.os = "windows";
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
