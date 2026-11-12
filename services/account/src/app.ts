// HTTP routes. Everything the handlers need is injected, so tests drive `handle` directly.

import type { Accounts } from "./accounts.ts";
import { AgentLoginError, proveAgentIdentity } from "./agent.ts";
import { checkClientRequest, ClientCodes, loopbackUrl } from "./client.ts";
import { accountPage, clientConfirmPage, clientHandoffPage, COPY_JS, errorPage, homePage, PAGE_HEADERS } from "./html.ts";
import { normalizeHandle, verifiedHandleForDid, type IdentityDeps } from "./identity.ts";
import { clientIp, RateLimiter } from "./ratelimit.ts";
import { WebSessions, type WebSession } from "./sessions.ts";
import type { Store } from "./db.ts";
import { IDENTITY_SCOPE } from "./oauth.ts";
import { resolveHandle } from "./identity.ts";

/** The slice of NodeOAuthClient the routes use. */
export interface OAuthPort {
  clientMetadata: unknown;
  jwks: unknown;
  authorize(handle: string, options: { scope: string; state?: string }): Promise<URL>;
  callback(params: URLSearchParams): Promise<{ session: { did: string; getTokenInfo(): Promise<{ scope: string }>; signOut(): Promise<void> }; state?: string | null }>;
}

export interface AppDeps {
  store: Store;
  publicUrl: string;
  accounts: Accounts;
  identity: IdentityDeps;
  oauth: OAuthPort;
  sessions: WebSessions;
  codes: ClientCodes;
  scope: string;
  realmAddress: string;
  loginsPerIp: RateLimiter;
  issuesPerDid: RateLimiter;
  log?: (message: string) => void;
}

const JSON_HEADERS = { "content-type": "application/json; charset=utf-8", "cache-control": "no-store" };

function json(body: unknown, status = 200, extra: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), { status, headers: { ...JSON_HEADERS, ...extra } });
}

function html(body: string, status = 200, extra: Record<string, string> = {}): Response {
  return new Response(body, { status, headers: { ...PAGE_HEADERS, ...extra } });
}

function redirect(location: string, extra: Record<string, string> = {}): Response {
  return new Response(null, { status: 303, headers: { location, "cache-control": "no-store", ...extra } });
}

function retryAfter(ms: number): Record<string, string> {
  return { "retry-after": String(Math.max(1, Math.ceil(ms / 1000))) };
}

/** After OAuth we only ever continue to the client sign-in page (re-validated there) or home. */
function safeNext(next: unknown): string | undefined {
  return typeof next === "string" && next.startsWith("/client/login?") && !next.includes("\\") ? next : undefined;
}

export function createApp(deps: AppDeps) {
  const log = deps.log ?? ((m: string) => console.log(m));
  const origin = new URL(deps.publicUrl).origin;
  // The SDK replaces sessions by DID. Keep callback replacement and consent inspection in one
  // serial lifecycle so a second callback cannot inspect/revoke the first callback's grant.
  let callbacks: Promise<unknown> = Promise.resolve();
  function serialCallback(url: URL): Promise<Response> {
    const run = callbacks.then(() => oauthCallback(url));
    callbacks = run.catch(() => {});
    return run;
  }

  /** Same-origin check for state-changing form posts, on top of the CSRF token. */
  function sameOrigin(req: Request): boolean {
    const o = req.headers.get("origin");
    return o === null || o === origin;
  }

  async function form(req: Request): Promise<Record<string, string>> {
    const data = await req.formData().catch(() => undefined);
    const out: Record<string, string> = {};
    data?.forEach((v, k) => {
      if (typeof v === "string") out[k] = v;
    });
    return out;
  }

  /** A session plus a valid CSRF token from the posted form, or an error response. */
  async function guardedPost(req: Request): Promise<{ session: WebSession; fields: Record<string, string> } | Response> {
    if (!sameOrigin(req)) return html(errorPage("Request from another site refused."), 403);
    const session = deps.sessions.get(req.headers.get("cookie"));
    if (!session) return redirect("/");
    const fields = await form(req);
    if (!WebSessions.csrfOk(session, fields.csrf)) return html(errorPage("This form expired. Go back and try again."), 403);
    return { session, fields };
  }

  async function home(req: Request): Promise<Response> {
    const session = deps.sessions.get(req.headers.get("cookie"));
    if (!session) return html(homePage());
    try {
      const a = await deps.accounts.ensure(session.did, session.handle);
      return html(accountPage({ handle: session.handle, account: a.account, password: a.password, realm: deps.realmAddress, csrf: session.csrf, publishing: deps.store.publishingEnabled(session.did) }));
    } catch (e) {
      log(`account view for ${session.did}: ${(e as Error).message}`);
      return html(errorPage("The realm could not look up your account right now. Try again shortly."), 503);
    }
  }

  async function reset(req: Request): Promise<Response> {
    const guarded = await guardedPost(req);
    if (guarded instanceof Response) return guarded;
    const { session } = guarded;
    const wait = deps.issuesPerDid.take(`did:${session.did}`);
    if (wait) return html(errorPage("Too many password resets. Try again later."), 429, retryAfter(wait));
    try {
      const a = await deps.accounts.reset(session.did, session.handle);
      log(`password reset: ${session.handle} (${session.did}) -> ${a.account}`);
      return html(
        accountPage({
          handle: session.handle,
          account: a.account,
          password: a.password,
          realm: deps.realmAddress,
          csrf: session.csrf,
          publishing: deps.store.publishingEnabled(session.did),
          notice: "New password set. The old one no longer works.",
        }),
      );
    } catch (e) {
      log(`password reset for ${session.did}: ${(e as Error).message}`);
      return html(errorPage("The realm could not reset your password right now. Try again shortly."), 503);
    }
  }

  async function logout(req: Request): Promise<Response> {
    const guarded = await guardedPost(req);
    if (guarded instanceof Response) return guarded;
    return redirect("/", { "set-cookie": deps.sessions.destroy(guarded.session) });
  }

  async function agentLogin(req: Request, ip: string): Promise<Response> {
    if (!(req.headers.get("content-type") ?? "").includes("application/json")) {
      return json({ error: "send application/json" }, 415);
    }
    const ipWait = deps.loginsPerIp.take(`ip:${ip}`);
    if (ipWait) return json({ error: "too many sign-ins from this address" }, 429, retryAfter(ipWait));
    let body: { handle?: unknown; app_password?: unknown; reset?: unknown };
    try {
      body = (await req.json()) as typeof body;
    } catch {
      return json({ error: "invalid JSON" }, 400);
    }
    if (body?.reset !== undefined && typeof body.reset !== "boolean") return json({ error: "reset must be a boolean" }, 400);
    try {
      const agent = await proveAgentIdentity(body?.handle, body?.app_password, deps.identity);
      const didWait = deps.issuesPerDid.take(`did:${agent.did}`);
      if (didWait) return json({ error: "too many sign-ins for this account" }, 429, retryAfter(didWait));
      const a = body.reset === true ? await deps.accounts.reset(agent.did, agent.handle) : await deps.accounts.ensure(agent.did, agent.handle);
      log(`agent login: ${agent.handle} (${agent.did}) -> ${a.account}${a.created ? " (new)" : a.passwordChanged ? " (new password)" : ""}`);
      return json({ account: a.account, password: a.password, realm: deps.realmAddress, did: agent.did, handle: agent.handle });
    } catch (e) {
      if (e instanceof AgentLoginError) return json({ error: e.message }, e.status);
      log(`agent login failed: ${(e as Error).message}`);
      return json({ error: "the realm could not issue an account right now" }, 503);
    }
  }

  async function oauthLogin(req: Request, ip: string): Promise<Response> {
    if (!sameOrigin(req)) return html(errorPage("Request from another site refused."), 403);
    const ipWait = deps.loginsPerIp.take(`ip:${ip}`);
    if (ipWait) return html(errorPage("Too many sign-ins from this address. Try again later."), 429);
    const fields = await form(req);
    const handle = normalizeHandle(fields.handle ?? "");
    if (!handle) return html(errorPage("That is not a valid handle (like alice.bsky.social)."), 400);
    const next = safeNext(fields.next);
    try {
      const did = await resolveHandle(handle, deps.identity);
      const state = JSON.stringify({ v: 1, did, version: deps.store.publishingVersion(did), publish: fields.publish === "yes", next });
      const url = await deps.oauth.authorize(handle, { scope: fields.publish === "yes" ? deps.scope : IDENTITY_SCOPE, state });
      return redirect(url.toString());
    } catch (e) {
      log(`oauth authorize ${handle}: ${(e as Error).message}`);
      return html(errorPage("Could not start sign-in with that handle's server."), 502);
    }
  }

  async function oauthCallback(url: URL): Promise<Response> {
    let did: string;
    let state: string | null | undefined;
    let oauthSession: Awaited<ReturnType<OAuthPort["callback"]>>["session"];
    try {
      const result = await deps.oauth.callback(url.searchParams);
      did = result.session.did;
      oauthSession = result.session;
      state = result.state;
    } catch (e) {
      log(`oauth callback: ${(e as Error).message}`);
      return html(errorPage("Sign-in was cancelled or could not be completed."), 400);
    }
    try {
      const handle = await verifiedHandleForDid(did, deps.identity);
      if (!handle) {
        deps.store.forgetOAuthSession(did);
        return html(errorPage("Your handle does not verify (its DNS or well-known record does not point at your account)."), 400);
      }
      // Only verified OAuth application state can grant publication consent. A stale callback
      // must not undo a withdrawal; legacy in-flight sign-ins fail closed to private.
      let consent: { v?: unknown; did?: unknown; version?: unknown; publish?: unknown; next?: unknown } = {};
      try { consent = JSON.parse(state ?? "{}"); } catch { /* legacy state is private */ }
      const valid = consent?.v === 1 && consent.did === did && Number.isSafeInteger(consent.version);
      const granted = (await oauthSession.getTokenInfo()).scope.split(" ");
      const publish = valid && consent.publish === true && deps.scope.split(" ").every((s) => granted.includes(s));
      const changed = deps.store.setPublishing(did, publish, valid ? consent.version as number : undefined);
      if (!changed || !publish) {
        await oauthSession.signOut().catch(() => {});
        deps.store.forgetOAuthSession(did);
      }
      // Make the account now, so the first page view (or the client) finds it ready.
      const a = await deps.accounts.ensure(did, handle);
      log(`oauth login: ${handle} (${did}) -> ${a.account}${a.created ? " (new)" : ""}`);
      const { setCookie } = deps.sessions.create(did, handle);
      return redirect(safeNext(valid ? consent.next : state) ?? "/", { "set-cookie": setCookie });
    } catch (e) {
      deps.store.forgetOAuthSession(did);
      log(`oauth sign-in for ${did}: ${(e as Error).message}`);
      return html(errorPage("The realm could not set up your account right now. Try again shortly."), 503);
    }
  }

  async function disablePublishing(req: Request): Promise<Response> {
    const guarded = await guardedPost(req);
    if (guarded instanceof Response) return guarded;
    deps.store.setPublishing(guarded.session.did, false);
    return redirect("/");
  }

  function clientLogin(req: Request, url: URL): Response {
    const check = checkClientRequest(url.searchParams);
    // Never redirect anywhere on a bad request: the redirect itself may be the bad part.
    if (!check.ok) return html(errorPage(`The client's sign-in request is invalid: ${check.error}.`), 400);
    const session = deps.sessions.get(req.headers.get("cookie"));
    if (!session) {
      return html(homePage(`/client/login?${url.searchParams}`, "Sign in with your Bluesky / ATProto account to sign in the WrathPlus client."));
    }
    const { redirect: r, state, challenge } = check.request;
    return html(clientConfirmPage(session.handle, { redirect: r, state, code_challenge: challenge, code_challenge_method: "S256" }, session.csrf));
  }

  async function clientApprove(req: Request): Promise<Response> {
    const guarded = await guardedPost(req);
    if (guarded instanceof Response) return guarded;
    const { session, fields } = guarded;
    const check = checkClientRequest(new URLSearchParams(fields));
    if (!check.ok) return html(errorPage(`The client's sign-in request is invalid: ${check.error}.`), 400);
    if (fields.decision !== "approve") return html(clientHandoffPage(loopbackUrl(check.request, origin, { error: "access_denied" }), false));
    try {
      await deps.accounts.ensure(session.did, session.handle); // the account exists before the client asks
    } catch (e) {
      log(`client approve for ${session.did}: ${(e as Error).message}`);
      return html(errorPage("The realm could not set up your account right now. Try again shortly."), 503);
    }
    const code = deps.codes.issue(session.did, session.handle, check.request);
    log(`client login approved: ${session.handle} (${session.did})`);
    return html(clientHandoffPage(loopbackUrl(check.request, origin, { code }), true));
  }

  async function clientToken(req: Request, ip: string): Promise<Response> {
    const ipWait = deps.loginsPerIp.take(`token:${ip}`);
    if (ipWait) return json({ error: "slow_down" }, 429, retryAfter(ipWait));
    const type = req.headers.get("content-type") ?? "";
    let body: Record<string, unknown> = {};
    if (type.includes("application/json")) body = ((await req.json().catch(() => ({}))) ?? {}) as Record<string, unknown>;
    else if (type.includes("application/x-www-form-urlencoded")) body = await form(req);
    else return json({ error: "invalid_request", error_description: "send application/json" }, 415);
    const redeemed = deps.codes.redeem(body.code, body.code_verifier);
    if (!redeemed.ok) return json({ error: redeemed.error }, 400);
    try {
      const a = await deps.accounts.ensure(redeemed.did, redeemed.handle);
      return json({ account: a.account, password: a.password, realm: deps.realmAddress, handle: redeemed.handle, did: redeemed.did });
    } catch (e) {
      log(`client token for ${redeemed.did}: ${(e as Error).message}`);
      return json({ error: "temporarily_unavailable" }, 503);
    }
  }

  return async function handle(req: Request, peer?: string): Promise<Response> {
    const url = new URL(req.url);
    const ip = clientIp(req.headers, peer);
    switch (`${req.method} ${url.pathname}`) {
      case "GET /":
        return home(req);
      case "GET /copy.js":
        return new Response(COPY_JS, {
          headers: { "content-type": "text/javascript; charset=utf-8", "cache-control": "public, max-age=300", "x-content-type-options": "nosniff" },
        });
      case "GET /healthz":
        return json({ ok: true });
      case "GET /oauth/client-metadata.json":
        return json(deps.oauth.clientMetadata, 200, { "cache-control": "public, max-age=300" });
      case "GET /oauth/jwks.json":
        return json(deps.oauth.jwks, 200, { "cache-control": "public, max-age=300" });
      case "POST /oauth/login":
        return oauthLogin(req, ip);
      case "GET /oauth/callback":
        return serialCallback(url);
      case "POST /account/privacy":
        return disablePublishing(req);
      case "POST /account/reset":
        return reset(req);
      case "POST /logout":
        return logout(req);
      case "GET /client/login":
        return clientLogin(req, url);
      case "POST /client/approve":
        return clientApprove(req);
      case "POST /client/token":
        return clientToken(req, ip);
      case "POST /api/agent/login":
        return agentLogin(req, ip);
      default:
        return json({ error: "not found" }, 404);
    }
  };
}
