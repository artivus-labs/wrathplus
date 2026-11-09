import { describe, expect, test } from "bun:test";
import { mkdtempSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Store } from "../src/db.ts";
import { depsFor, fakeNet } from "./fakes.ts";
import { clientMetadata, createOAuthClient, guardedFetch, loadOrCreateKeys, OAUTH_SCOPE } from "../src/oauth.ts";

describe("OAuth client", () => {
  const dir = mkdtempSync(join(tmpdir(), "wp-oauth-"));
  const keysPath = join(dir, "oauth-keys.json");

  test("keys are generated once into a 0600 file and reloaded", async () => {
    const first = await loadOrCreateKeys(keysPath);
    expect(statSync(keysPath).mode & 0o777).toBe(0o600);
    const again = await loadOrCreateKeys(keysPath);
    expect(again[0]!.kid).toBe(first[0]!.kid);
    expect(first[0]!.algorithms).toContain("ES256");
  });

  test("scope is atproto plus our two collections only", () => {
    expect(OAUTH_SCOPE.split(" ")).toEqual([
      "atproto",
      "repo:au.com.artivus.wrathplus.character?action=create&action=update",
      "repo:au.com.artivus.wrathplus.milestone?action=create&action=update",
    ]);
  });

  test("the library accepts our metadata; the published JWKS has no private parts", async () => {
    const keys = await loadOrCreateKeys(keysPath);
    const client = createOAuthClient("https://wrathplus.artivus.com.au", keys, new Store(":memory:"), depsFor(fakeNet()));
    const meta = client.clientMetadata as Record<string, unknown>;
    expect(meta.client_id).toBe("https://wrathplus.artivus.com.au/oauth/client-metadata.json");
    expect(meta.redirect_uris).toEqual(["https://wrathplus.artivus.com.au/oauth/callback"]);
    expect(meta.token_endpoint_auth_method).toBe("private_key_jwt");
    expect(meta.dpop_bound_access_tokens).toBe(true);
    const jwks = client.jwks as unknown as { keys: Array<Record<string, unknown>> };
    expect(jwks.keys.length).toBe(1);
    expect(jwks.keys[0]!.d).toBeUndefined();
    expect(clientMetadata("https://a.example/").jwks_uri).toBe("https://a.example/oauth/jwks.json");
  });

  test("state store expires entries; session store round-trips", async () => {
    let now = 0;
    const store = new Store(":memory:");
    const state = store.stateStore<{ x: number }>(1000, () => now);
    await state.set("k", { x: 1 });
    expect(await state.get("k")).toEqual({ x: 1 });
    now = 2000;
    expect(await state.get("k")).toBeUndefined();
    const sessions = store.sessionStore<{ y: string }>();
    await sessions.set("did:plc:x", { y: "z" });
    expect(store.hasSession("did:plc:x")).toBe(true);
    await sessions.del("did:plc:x");
    expect(await sessions.get("did:plc:x")).toBeUndefined();
  });
});

describe("guardedFetch (the OAuth library's fetch)", () => {
  test("refuses loopback, private and plain-http targets before any request", async () => {
    const net = fakeNet();
    net.hosts["pds.example.com"] = ["203.0.113.10"];
    net.routes["GET https://pds.example.com/.well-known/oauth-protected-resource"] = () => Response.json({ ok: true });
    const f = guardedFetch(depsFor(net));
    expect((await f("https://pds.example.com/.well-known/oauth-protected-resource")).status).toBe(200);
    for (const url of ["http://pds.example.com/", "https://127.0.0.1:17878/", "https://[::1]/x", "https://unresolvable.example.com/"]) {
      await expect(f(url)).rejects.toThrow();
    }
    expect(net.requests.length).toBe(1);
  });
});
