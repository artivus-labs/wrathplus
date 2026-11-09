// The three pages people see: sign in, your account (password shown once), and an error.
// Plain server-rendered HTML, no scripts.

export function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&#39;");
}

export const PAGE_HEADERS = {
  "content-type": "text/html; charset=utf-8",
  "cache-control": "no-store",
  "referrer-policy": "no-referrer",
  "x-content-type-options": "nosniff",
  "x-frame-options": "DENY",
  // form-action allows https: because the sign-in form's redirect goes to the person's own
  // authorization server.
  "content-security-policy": "default-src 'none'; style-src 'unsafe-inline'; form-action 'self' https:; frame-ancestors 'none'; base-uri 'none'",
};

const STYLE = `
  :root { color-scheme: dark; --bg: #0d1117; --card: #161b22; --line: #30363d; --fg: #e6edf3; --dim: #8b949e; --gold: #e3b341; --ice: #79c0ff; }
  * { box-sizing: border-box; }
  body { margin: 0; min-height: 100vh; display: grid; place-items: center; background: radial-gradient(1200px 600px at 50% -10%, #1f2a3a 0%, var(--bg) 60%); color: var(--fg); font: 16px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif; }
  main { width: min(30rem, 92vw); background: var(--card); border: 1px solid var(--line); border-radius: 14px; padding: 2rem; box-shadow: 0 20px 60px #0008; }
  h1 { margin: 0 0 .25rem; font: 600 1.6rem/1.2 Georgia, "Times New Roman", serif; letter-spacing: .02em; color: var(--gold); }
  p { margin: .5rem 0; color: var(--dim); }
  label { display: block; margin: 1.25rem 0 .4rem; font-size: .9rem; color: var(--dim); }
  input { width: 100%; padding: .7rem .8rem; border-radius: 8px; border: 1px solid var(--line); background: var(--bg); color: var(--fg); font-size: 1rem; }
  button { margin-top: 1rem; width: 100%; padding: .75rem; border: 0; border-radius: 8px; background: var(--ice); color: #04121f; font-weight: 600; font-size: 1rem; cursor: pointer; }
  dl { margin: 1.25rem 0 0; display: grid; grid-template-columns: auto 1fr; gap: .6rem 1rem; }
  dt { color: var(--dim); }
  dd { margin: 0; font-family: ui-monospace, "SF Mono", Menlo, monospace; word-break: break-all; }
  .secret { font-size: 1.15rem; color: var(--gold); user-select: all; }
  .note { margin-top: 1.25rem; padding: .75rem .9rem; border-left: 3px solid var(--gold); background: #e3b3410f; color: var(--fg); font-size: .92rem; }
  a { color: var(--ice); }
  small { color: var(--dim); }
`;

function page(title: string, body: string): string {
  return `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex"><title>${escapeHtml(title)}</title><style>${STYLE}</style></head>
<body><main>${body}</main></body></html>`;
}

export function homePage(): string {
  return page(
    "WrathPlus beta: sign in",
    `<h1>WrathPlus</h1>
<p>Open beta realm. Sign in with your Bluesky / ATProto account to get your game account.</p>
<form method="post" action="/oauth/login">
  <label for="handle">Your handle</label>
  <input id="handle" name="handle" placeholder="alice.bsky.social" autocomplete="username" autocapitalize="none" spellcheck="false" required>
  <button type="submit">Sign in</button>
</form>
<p><small>Agents: <code>POST /api/agent/login</code> with your handle and an app password. See the project README.</small></p>`,
  );
}

export function accountPage(handle: string, account: string, password: string, realm: string): string {
  return page(
    "WrathPlus beta: your account",
    `<h1>Welcome, @${escapeHtml(handle)}</h1>
<p>Your game account is ready.</p>
<dl>
  <dt>Account</dt><dd>${escapeHtml(account)}</dd>
  <dt>Password</dt><dd class="secret">${escapeHtml(password)}</dd>
  <dt>Realm</dt><dd>${escapeHtml(realm)}</dd>
</dl>
<div class="note">This password is shown once. Copy it now. Signing in here again issues a new one, and the old one stops working.</div>
<p><small>Passwords are not case-sensitive in game.</small></p>`,
  );
}

export function errorPage(message: string): string {
  return page("WrathPlus beta: sign-in failed", `<h1>Sign-in failed</h1><p>${escapeHtml(message)}</p><p><a href="/">Try again</a></p>`);
}
