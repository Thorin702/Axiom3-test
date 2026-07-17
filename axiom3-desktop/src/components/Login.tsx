// Account login gate. The app authenticates against the website's account system (via the Rust
// `account_login` command) before the workbench is shown. No account is created here — accounts
// come from the website (register → admin approval → validity period).
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type Lang = "zh" | "en";
export type Auth = { account: string; expiry: string | null };

const MARK = (
  <svg viewBox="5 5 44 44" width="46" height="46" aria-hidden>
    <path
      d="M13 15 L30 11 M30 11 L41 21 M13 15 L24 30 M30 11 L24 30 M41 21 L38 35 M24 30 L38 35 M24 30 L16 42 M38 35 L31 43 M16 42 L31 43"
      fill="none" stroke="var(--ax-accent)" strokeWidth="1.5" strokeLinecap="round" opacity=".45"
    />
    <g fill="var(--ax-accent)">
      <circle cx="13" cy="15" r="3.3" /><circle cx="30" cy="11" r="3.3" /><circle cx="41" cy="21" r="3.3" />
      <circle cx="24" cy="30" r="4.3" /><circle cx="38" cy="35" r="3.3" /><circle cx="16" cy="42" r="3.3" /><circle cx="31" cy="43" r="3.3" />
    </g>
  </svg>
);

export function Login({ lang, msg, onLogin }: { lang: Lang; msg: string | null; onLogin: (a: Auth) => void }) {
  const zh = lang === "zh";
  const [account, setAccount] = useState("");
  const [password, setPassword] = useState("");
  const [err, setErr] = useState<string | null>(msg);
  const [busy, setBusy] = useState(false);

  async function submit(e?: React.FormEvent) {
    e?.preventDefault();
    if (!account.trim() || !password || busy) return;
    setBusy(true);
    setErr(null);
    try {
      const r = await invoke<{ ok: boolean; account?: string; expiry?: string; error?: string }>("account_login", {
        account: account.trim(),
        password,
      });
      if (r.ok) onLogin({ account: r.account || account.trim(), expiry: r.expiry ?? null });
      else setErr(r.error || (zh ? "登录失败" : "Login failed"));
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="ax-login">
      <form className="ax-login__card" onSubmit={submit}>
        <div className="ax-login__brand">
          {MARK}
          <span>Axiom3</span>
        </div>
        <p className="ax-login__sub">{zh ? "用官网账号登录" : "Sign in with your website account"}</p>
        <label className="ax-login__field">
          <span>{zh ? "账号" : "Account"}</span>
          <input value={account} onChange={(e) => setAccount(e.target.value)} autoFocus autoComplete="username" spellCheck={false} />
        </label>
        <label className="ax-login__field">
          <span>{zh ? "密码" : "Password"}</span>
          <input type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="current-password" />
        </label>
        {err && <div className="ax-login__err">{err}</div>}
        <button className="ax-login__btn" type="submit" disabled={busy || !account.trim() || !password}>
          {busy ? (zh ? "登录中…" : "Signing in…") : zh ? "登录" : "Sign in"}
        </button>
        <div className="ax-login__hint">
          {zh ? "没有账号?请到官网注册,等待审批开通后再登录。App 内不单独注册。" : "No account? Register on the website and wait for approval."}
        </div>
      </form>
    </div>
  );
}
