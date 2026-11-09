// ATProto OAuth for people: a confidential client (private_key_jwt, ES256) published at
// <PUBLIC_URL>/oauth/client-metadata.json. The signing keys are generated on first start into a
// 0600 JSON file; the state and session stores live in the service's SQLite file.

import { closeSync, existsSync, openSync, readFileSync, statSync, writeSync } from "node:fs";
import { JoseKey, NodeOAuthClient, requestLocalLock, type NodeSavedSession, type NodeSavedState } from "@atproto/oauth-client-node";
import type { Store } from "./db.ts";
import { assertPublicUrl, resolveHandle, type IdentityDeps } from "./identity.ts";
import { NSID } from "./records.ts";

/** `atproto` (identity) plus create/update on our two collections, nothing else. */
export const OAUTH_SCOPE = ["atproto", `repo:${NSID.character}?action=create&action=update`, `repo:${NSID.milestone}?action=create&action=update`].join(" ");

export function clientMetadata(publicUrl: string) {
  const base = publicUrl.replace(/\/$/, "");
  return {
    client_id: `${base}/oauth/client-metadata.json`,
    client_name: "WrathPlus",
    client_uri: base,
    redirect_uris: [`${base}/oauth/callback`] as [string],
    grant_types: ["authorization_code", "refresh_token"] as ["authorization_code", "refresh_token"],
    response_types: ["code"] as ["code"],
    scope: OAUTH_SCOPE,
    application_type: "web" as const,
    token_endpoint_auth_method: "private_key_jwt" as const,
    token_endpoint_auth_signing_alg: "ES256",
    dpop_bound_access_tokens: true,
    jwks_uri: `${base}/oauth/jwks.json`,
  };
}

/** The private signing keys: loaded from `path`, or generated into it (mode 0600) on first start. */
export async function loadOrCreateKeys(path: string): Promise<JoseKey[]> {
  if (!existsSync(path)) {
    const key = await JoseKey.generate(["ES256"], `wrathplus-${new Date().toISOString().slice(0, 10)}`);
    // O_EXCL: two starts racing never overwrite each other's key.
    const fd = openSync(path, "wx", 0o600);
    try {
      writeSync(fd, JSON.stringify({ keys: [key.privateJwk] }, null, 2));
    } finally {
      closeSync(fd);
    }
  }
  if (statSync(path).mode & 0o077) throw new Error(`${path} must not be readable by group or others (chmod 600)`);
  const { keys } = JSON.parse(readFileSync(path, "utf8")) as { keys: Array<Record<string, unknown> & { kid?: string }> };
  if (!Array.isArray(keys) || keys.length === 0) throw new Error(`${path} holds no keys`);
  return Promise.all(keys.map((jwk) => JoseKey.fromImportable(jwk as never, jwk.kid)));
}

export function createOAuthClient(publicUrl: string, keys: JoseKey[], store: Store, identity: IdentityDeps): NodeOAuthClient {
  return new NodeOAuthClient({
    // The library's default handle resolver and SSRF guard are built on undici dispatchers, which
    // Bun does not provide ("Unicast SSRF protection requires Node.js 20.6+"). Ours do the same
    // checks: https only, public addresses only, DNS TXT then well-known.
    handleResolver: { resolve: async (handle: string) => resolveHandle(handle, identity).catch(() => null) },
    fetch: guardedFetch(identity),
    clientMetadata: clientMetadata(publicUrl),
    keyset: keys,
    stateStore: store.stateStore<NodeSavedState>(),
    sessionStore: store.sessionStore<NodeSavedSession>(),
    requestLock: requestLocalLock,
  });
}

/** fetch for the OAuth library: every URL (PDS, authorization server, did:web) must be public https. */
export function guardedFetch(identity: Pick<IdentityDeps, "fetch" | "lookup">): typeof fetch {
  const f = async (input: string | URL | Request, init?: RequestInit) => {
    const request = (input instanceof Request ? new Request(input, init) : new Request(input.toString(), init));
    await assertPublicUrl(request.url, identity);
    return identity.fetch(request, { redirect: "error" });
  };
  return f as typeof fetch;
}
