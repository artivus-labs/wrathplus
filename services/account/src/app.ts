// HTTP routes. Everything the handlers need is injected, so tests drive `handle` directly.

import type { Accounts } from "./accounts.ts";
import { AgentLoginError, proveAgentIdentity } from "./agent.ts";
import { accountPage, errorPage, homePage, PAGE_HEADERS } from "./html.ts";
import { normalizeHandle, verifiedHandleForDid, type IdentityDeps } from "./identity.ts";
import { clientIp, RateLimiter } from "./ratelimit.ts";

/** The slice of NodeOAuthClient the routes use. */
export interface OAuthPort {
  clientMetadata: unknown;
  jwks: unknown;
  authorize(handle: string, options: { scope: string }): Promise<URL>;
  callback(params: URLSearchParams): Promise<{ session: { did: string } }>;
}

export interface AppDeps {
  accounts: Accounts;
  identity: IdentityDeps;
  oauth: OAuthPort;
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

function html(body: string, status = 200): Response {
  return new Response(body, { status, headers: PAGE_HEADERS });
}

function retryAfter(ms: number): Record<string, string> {
  return { "retry-after": String(Math.max(1, Math.ceil(ms / 1000))) };
}

export function createApp(deps: AppDeps) {
  const log = deps.log ?? ((m: string) => console.log(m));

  async function agentLogin(req: Request, ip: string): Promise<Response> {
    if (!(req.headers.get("content-type") ?? "").includes("application/json")) {
      return json({ error: "send application/json" }, 415);
    }
    const ipWait = deps.loginsPerIp.take(`ip:${ip}`);
    if (ipWait) return json({ error: "too many sign-ins from this address" }, 429, retryAfter(ipWait));
    let body: { handle?: unknown; app_password?: unknown };
    try {
      body = (await req.json()) as typeof body;
    } catch {
      return json({ error: "invalid JSON" }, 400);
    }
    try {
      const agent = await proveAgentIdentity(body?.handle, body?.app_password, deps.identity);
      const didWait = deps.issuesPerDid.take(`did:${agent.did}`);
      if (didWait) return json({ error: "too many sign-ins for this account" }, 429, retryAfter(didWait));
      const issued = await deps.accounts.issue(agent.did, agent.handle);
      log(`agent login: ${agent.handle} (${agent.did}) -> ${issued.account}${issued.created ? " (new)" : ""}`);
      return json({ account: issued.account, password: issued.password, realm: deps.realmAddress, did: agent.did, handle: agent.handle });
    } catch (e) {
      if (e instanceof AgentLoginError) return json({ error: e.message }, e.status);
      log(`agent login failed: ${(e as Error).message}`);
      return json({ error: "the realm could not issue an account right now" }, 503);
    }
  }

  async function oauthLogin(req: Request, ip: string): Promise<Response> {
    const ipWait = deps.loginsPerIp.take(`ip:${ip}`);
    if (ipWait) return html(errorPage("Too many sign-ins from this address. Try again later."), 429);
    const form = await req.formData().catch(() => undefined);
    const raw = form?.get("handle");
    const handle = typeof raw === "string" ? normalizeHandle(raw) : undefined;
    if (!handle) return html(errorPage("That is not a valid handle (like alice.bsky.social)."), 400);
    try {
      const url = await deps.oauth.authorize(handle, { scope: deps.scope });
      return new Response(null, { status: 303, headers: { location: url.toString(), "cache-control": "no-store" } });
    } catch (e) {
      log(`oauth authorize ${handle}: ${(e as Error).message}`);
      return html(errorPage("Could not start sign-in with that handle's server."), 502);
    }
  }

  async function oauthCallback(url: URL): Promise<Response> {
    let did: string;
    try {
      ({ did } = (await deps.oauth.callback(url.searchParams)).session);
    } catch (e) {
      log(`oauth callback: ${(e as Error).message}`);
      return html(errorPage("Sign-in was cancelled or could not be completed."), 400);
    }
    const didWait = deps.issuesPerDid.take(`did:${did}`);
    if (didWait) return html(errorPage("Too many sign-ins for this account. Try again later."), 429);
    try {
      const handle = await verifiedHandleForDid(did, deps.identity);
      if (!handle) return html(errorPage("Your handle does not verify (its DNS or well-known record does not point at your account)."), 400);
      const issued = await deps.accounts.issue(did, handle);
      log(`oauth login: ${handle} (${did}) -> ${issued.account}${issued.created ? " (new)" : ""}`);
      return html(accountPage(handle, issued.account, issued.password, deps.realmAddress));
    } catch (e) {
      log(`oauth issue for ${did}: ${(e as Error).message}`);
      return html(errorPage("The realm could not issue your account right now. Try again shortly."), 503);
    }
  }

  return async function handle(req: Request, peer?: string): Promise<Response> {
    const url = new URL(req.url);
    const ip = clientIp(req.headers, peer);
    const route = `${req.method} ${url.pathname}`;
    switch (route) {
      case "GET /":
        return html(homePage());
      case "GET /healthz":
        return json({ ok: true });
      case "GET /oauth/client-metadata.json":
        return json(deps.oauth.clientMetadata, 200, { "cache-control": "public, max-age=300" });
      case "GET /oauth/jwks.json":
        return json(deps.oauth.jwks, 200, { "cache-control": "public, max-age=300" });
      case "POST /oauth/login":
        return oauthLogin(req, ip);
      case "GET /oauth/callback":
        return oauthCallback(url);
      case "POST /api/agent/login":
        return agentLogin(req, ip);
      default:
        return json({ error: "not found" }, 404);
    }
  };
}
