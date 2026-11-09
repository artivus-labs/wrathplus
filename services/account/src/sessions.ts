// Web sessions for people, after ATProto OAuth. The cookie carries a random id plus an HMAC of it
// (`__Host-` prefix: Secure, Path=/, no Domain; HttpOnly; SameSite=Lax). The server keeps only
// the id's hash, the DID, the verified handle and the session's CSRF token, so a session can be
// ended server-side and a leaked database does not yield usable cookies.

import { createHash } from "node:crypto";
import type { Store } from "./db.ts";
import { hmac, randomToken, safeEqual } from "./secrets.ts";

export const SESSION_COOKIE = "__Host-wp_session";
export const SESSION_TTL_MS = 30 * 24 * 60 * 60 * 1000;

export interface WebSession {
  idHash: string;
  did: string;
  handle: string;
  csrf: string;
}

const sha256 = (s: string) => createHash("sha256").update(s).digest("base64url");

export function readCookie(header: string | null, name: string): string | undefined {
  if (!header) return undefined;
  for (const part of header.split(";")) {
    const at = part.indexOf("=");
    if (at > 0 && part.slice(0, at).trim() === name) return part.slice(at + 1).trim();
  }
  return undefined;
}

export class WebSessions {
  constructor(
    private readonly store: Store,
    private readonly key: Buffer,
    private readonly now: () => number = () => Date.now(),
    private readonly ttlMs = SESSION_TTL_MS,
  ) {}

  /** A new session; returns the Set-Cookie header value. */
  create(did: string, handle: string): { setCookie: string; session: WebSession } {
    const id = randomToken();
    const session = { idHash: sha256(id), did, handle, csrf: randomToken() };
    const now = this.now();
    this.store.insertWebSession(session.idHash, did, handle, session.csrf, now + this.ttlMs, now);
    const value = `${id}.${hmac(this.key, id)}`;
    const setCookie = `${SESSION_COOKIE}=${value}; Path=/; Max-Age=${Math.floor(this.ttlMs / 1000)}; HttpOnly; Secure; SameSite=Lax`;
    return { setCookie, session };
  }

  /** The request's session, if its cookie is genuine and the session is live. */
  get(cookieHeader: string | null): WebSession | undefined {
    const value = readCookie(cookieHeader, SESSION_COOKIE);
    if (!value) return undefined;
    const dot = value.lastIndexOf(".");
    if (dot <= 0) return undefined;
    const id = value.slice(0, dot);
    if (!safeEqual(value.slice(dot + 1), hmac(this.key, id))) return undefined;
    const idHash = sha256(id);
    const row = this.store.webSession(idHash);
    if (!row) return undefined;
    if (row.expiresAt < this.now()) {
      this.store.deleteWebSession(idHash);
      return undefined;
    }
    return { idHash, did: row.did, handle: row.handle, csrf: row.csrf };
  }

  destroy(session: WebSession): string {
    this.store.deleteWebSession(session.idHash);
    return `${SESSION_COOKIE}=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Lax`;
  }

  /** The form's CSRF token matches the session's. */
  static csrfOk(session: WebSession, token: unknown): boolean {
    return typeof token === "string" && safeEqual(token, session.csrf);
  }
}
