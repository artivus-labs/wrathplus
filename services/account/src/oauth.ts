// ATProto OAuth for people: a confidential client (private_key_jwt, ES256) published at
// <PUBLIC_URL>/oauth/client-metadata.json. The signing keys are generated on first start into a
// 0600 JSON file; the state and session stores live in the service's SQLite file.

import { closeSync, existsSync, openSync, readFileSync, statSync, writeSync } from "node:fs";
import { JoseKey, NodeOAuthClient, requestLocalLock, type NodeSavedSession, type NodeSavedState } from "@atproto/oauth-client-node";
import type { Store } from "./db.ts";
import { assertPublicUrl, resolveHandle, type IdentityDeps } from "./identity.ts";
import { NSID } from "./records.ts";

/** Identity alone never grants permission to publish gameplay. */
export const IDENTITY_SCOPE = "atproto";
/** Maximum supported scope; requested only after explicit public-record consent. */
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
  const client = new NodeOAuthClient({
    // The library's default handle resolver and SSRF guard are built on undici dispatchers, which
    // Bun does not provide ("Unicast SSRF protection requires Node.js 20.6+"). Ours do the same
    // checks: https only, public addresses only, DNS TXT then well-known.
    handleResolver: { resolve: async (handle: string) => resolveHandle(handle, identity).catch(() => null) },
    fetch: guardedFetch(identity, (did) => store.publishingEnabled(did)),
    clientMetadata: clientMetadata(publicUrl),
    keyset: keys,
    stateStore: store.stateStore<NodeSavedState>(),
    sessionStore: store.sessionStore<NodeSavedSession>(),
    requestLock: requestLocalLock,
  });
  const callback = client.callback.bind(client);
  client.callback = async (params, options) => {
    // Check saved application state before the SDK revokes/replaces the DID's current session.
    const key = params.get("state");
    const saved = key ? await store.stateStore<NodeSavedState>().get(key) : undefined;
    if (saved && !currentConsentState(saved.appState, store)) {
      await store.stateStore().del(key!);
      throw new Error("Sign-in superseded; start again");
    }
    return callback(params, options);
  };
  return client;
}

/** Legacy states may sign in privately; versioned consent callbacks must still be current. */
export function currentConsentState(state: unknown, store: Store): boolean {
  if (typeof state !== "string") return true;
  try {
    const parsed = JSON.parse(state);
    if (parsed?.v !== 1) return true;
    return typeof parsed.did === "string" && Number.isSafeInteger(parsed.version) && store.publishingVersion(parsed.did) === parsed.version;
  } catch { return true; }
}

/** fetch for the OAuth library: every URL (PDS, authorization server, did:web) must be public https. */
export function guardedFetch(identity: Pick<IdentityDeps, "fetch" | "lookup">, canPublish: (did: string) => boolean = () => false): typeof fetch {
  const f = async (input: string | URL | Request, init?: RequestInit) => {
    const request = (input instanceof Request ? new Request(input, init) : new Request(input.toString(), init));
    const recordWrite = new URL(request.url).pathname === "/xrpc/com.atproto.repo.putRecord";
    const repo = recordWrite ? (await request.clone().json() as { repo?: unknown }).repo : undefined;
    await assertPublicUrl(request.url, identity);
    // The OAuth library may refresh asynchronously inside fetchHandler. Check again at the
    // actual network boundary, after DNS validation, so withdrawal also fences that interval.
    if (recordWrite && (typeof repo !== "string" || !canPublish(repo))) throw new Error("Public record consent withdrawn");
    return identity.fetch(request, { redirect: "error" });
  };
  return f as typeof fetch;
}
