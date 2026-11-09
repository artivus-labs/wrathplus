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
  button.quiet { background: transparent; color: var(--dim); border: 1px solid var(--line); }
  .row { display: flex; gap: .75rem; }
  .row form { flex: 1; }
  details { margin-top: .25rem; }
  summary { cursor: pointer; color: var(--ice); }
  a { color: var(--ice); }
  small { color: var(--dim); }
`;

function page(title: string, body: string): string {
  return `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex"><title>${escapeHtml(title)}</title><style>${STYLE}</style></head>
<body><main>${body}</main></body></html>`;
}

const hidden = (name: string, value: string) => `<input type="hidden" name="${escapeHtml(name)}" value="${escapeHtml(value)}">`;

/** Sign in. `next` is where to continue afterwards (only the client sign-in uses it). */
export function homePage(next?: string, intro?: string): string {
  return page(
    "WrathPlus beta: sign in",
    `<h1>WrathPlus</h1>
<p>${escapeHtml(intro ?? "Open beta realm. Sign in with your Bluesky / ATProto account to get your game account.")}</p>
<form method="post" action="/oauth/login">
  <label for="handle">Your handle</label>
  <input id="handle" name="handle" placeholder="alice.bsky.social" autocomplete="username" autocapitalize="none" spellcheck="false" required>
  ${next ? hidden("next", next) : ""}
  <button type="submit">Sign in</button>
</form>
<p><small>Agents: <code>POST /api/agent/login</code> with your handle and an app password. See the project README.</small></p>`,
  );
}

export interface AccountView {
  handle: string;
  account: string;
  password: string;
  realm: string;
  csrf: string;
  notice?: string;
}

export function accountPage(v: AccountView): string {
  return page(
    "WrathPlus beta: your account",
    `<h1>@${escapeHtml(v.handle)}</h1>
<p>Your game account on the beta realm.</p>
${v.notice ? `<div class="note">${escapeHtml(v.notice)}</div>` : ""}
<dl>
  <dt>Account</dt><dd>${escapeHtml(v.account)}</dd>
  <dt>Password</dt><dd><details><summary>Show</summary><span class="secret">${escapeHtml(v.password)}</span></details></dd>
  <dt>Realm</dt><dd>${escapeHtml(v.realm)}</dd>
</dl>
<p><small>Passwords are not case-sensitive in game.</small></p>
<div class="row">
  <form method="post" action="/account/reset">${hidden("csrf", v.csrf)}<button type="submit">Reset password</button></form>
  <form method="post" action="/logout">${hidden("csrf", v.csrf)}<button class="quiet" type="submit">Sign out</button></form>
</div>`,
  );
}

export function clientConfirmPage(handle: string, fields: Record<string, string>, csrf: string): string {
  const all = Object.entries({ ...fields, csrf })
    .map(([k, val]) => hidden(k, val))
    .join("");
  return page(
    "WrathPlus beta: sign in the client",
    `<h1>Sign in the client</h1>
<p>Sign in WrathPlus client as <strong>@${escapeHtml(handle)}</strong>?</p>
<p><small>The client on this computer receives your game account and password. Continue only if you just started a sign-in from the WrathPlus client or launcher.</small></p>
<div class="row">
  <form method="post" action="/client/approve">${all}${hidden("decision", "approve")}<button type="submit">Sign in</button></form>
  <form method="post" action="/client/approve">${all}${hidden("decision", "deny")}<button class="quiet" type="submit">Cancel</button></form>
</div>
<p><small>Not @${escapeHtml(handle)}? <a href="/">Sign out</a> first.</small></p>`,
  );
}

/**
 * Hands the browser to the client's loopback listener. A meta refresh rather than a redirect from
 * the form's POST: CSP form-action would otherwise have to allow http://127.0.0.1:* and [::1].
 */
export function clientHandoffPage(url: string, approved: boolean): string {
  const u = escapeHtml(url);
  return page(
    "WrathPlus beta: back to the client",
    `<meta http-equiv="refresh" content="0;url=${u}">
<h1>${approved ? "Signed in" : "Cancelled"}</h1>
<p>${approved ? "Returning to the WrathPlus client. You can close this tab." : "The client was not signed in."}</p>
<p><a href="${u}">Continue to the client</a></p>`,
  );
}

export function errorPage(message: string): string {
  return page("WrathPlus beta: sign-in failed", `<h1>Sign-in failed</h1><p>${escapeHtml(message)}</p><p><a href="/">Back</a></p>`);
}
