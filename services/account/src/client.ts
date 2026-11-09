// Browser sign-in for the WrathPlus client and launcher: an OAuth-style loopback flow
// (RFC 8252 loopback redirect + RFC 7636 PKCE, S256 only). See CLIENT-LOGIN.md.
//
//   GET  /client/login?redirect=http://127.0.0.1:<port>/callback&state=…&code_challenge=…
//   (person signs in, or already has a session, and confirms)
//   -> http://127.0.0.1:<port>/callback?code=…&state=…&iss=…
//   POST /client/token {code, code_verifier} -> {account, password, realm, handle, did}
//
// Codes are random 256-bit values, stored only as hashes, valid for 60 seconds and taken from
// the store on the first redemption attempt, right or wrong.

import { createHash } from "node:crypto";
import type { Store } from "./db.ts";
import { randomToken, safeEqual } from "./secrets.ts";

export const CODE_TTL_MS = 60_000;
const STATE = /^[A-Za-z0-9._~-]{16,256}$/;
const CHALLENGE = /^[A-Za-z0-9_-]{43}$/;
const VERIFIER = /^[A-Za-z0-9._~-]{43,128}$/;
const PATH = /^\/[A-Za-z0-9._~\/-]{0,200}$/;

export interface ClientRequest {
  redirect: string;
  state: string;
  challenge: string;
}

export type ClientRequestCheck = { ok: true; request: ClientRequest } | { ok: false; error: string };

/**
 * Only a loopback http redirect is accepted: host 127.0.0.1 or [::1] (never "localhost", which
 * can resolve elsewhere), an explicit port, a plain path, no query, fragment or credentials.
 */
export function checkRedirect(raw: string): string | undefined {
  let u: URL;
  try {
    u = new URL(raw);
  } catch {
    return undefined;
  }
  if (u.protocol !== "http:") return undefined;
  if (u.hostname !== "127.0.0.1" && u.hostname !== "[::1]") return undefined;
  if (!u.port) return undefined;
  if (u.username || u.password || u.search || u.hash || raw.includes("#") || raw.includes("?")) return undefined;
  if (!PATH.test(u.pathname)) return undefined;
  return `${u.protocol}//${u.host}${u.pathname}`;
}

export function checkClientRequest(params: URLSearchParams): ClientRequestCheck {
  const redirect = checkRedirect(params.get("redirect") ?? "");
  if (!redirect) return { ok: false, error: "redirect must be http://127.0.0.1:<port>/<path> or http://[::1]:<port>/<path>" };
  const state = params.get("state") ?? "";
  if (!STATE.test(state)) return { ok: false, error: "state must be 16 to 256 URL-safe characters" };
  const method = params.get("code_challenge_method") ?? "S256";
  if (method !== "S256") return { ok: false, error: "code_challenge_method must be S256" };
  const challenge = params.get("code_challenge") ?? "";
  if (!CHALLENGE.test(challenge)) return { ok: false, error: "code_challenge must be BASE64URL(SHA-256(code_verifier)), 43 characters" };
  return { ok: true, request: { redirect, state, challenge } };
}

export function s256(verifier: string): string {
  return createHash("sha256").update(verifier, "ascii").digest("base64url");
}

const hashCode = (code: string) => createHash("sha256").update(code).digest("base64url");

/** Where the browser goes after approval (or denial). */
export function loopbackUrl(request: ClientRequest, issuer: string, result: { code: string } | { error: string }): string {
  const q = new URLSearchParams("code" in result ? { code: result.code } : { error: result.error });
  q.set("state", request.state);
  q.set("iss", issuer);
  return `${request.redirect}?${q}`;
}

export type Redeemed = { ok: true; did: string; handle: string } | { ok: false; error: "invalid_request" | "invalid_grant" };

export class ClientCodes {
  constructor(
    private readonly store: Store,
    private readonly now: () => number = () => Date.now(),
  ) {}

  issue(did: string, handle: string, request: ClientRequest): string {
    const code = randomToken();
    const now = this.now();
    this.store.insertClientCode(hashCode(code), did, handle, request.challenge, request.redirect, now + CODE_TTL_MS, now);
    return code;
  }

  redeem(code: unknown, verifier: unknown): Redeemed {
    if (typeof code !== "string" || typeof verifier !== "string" || code.length === 0 || code.length > 128) return { ok: false, error: "invalid_request" };
    if (!VERIFIER.test(verifier)) return { ok: false, error: "invalid_request" };
    const row = this.store.takeClientCode(hashCode(code)); // single use: gone from here on
    if (!row) return { ok: false, error: "invalid_grant" };
    if (row.expiresAt < this.now()) return { ok: false, error: "invalid_grant" };
    if (!safeEqual(s256(verifier), row.challenge)) return { ok: false, error: "invalid_grant" };
    return { ok: true, did: row.did, handle: row.handle };
  }
}
