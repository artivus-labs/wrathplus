import { describe, expect, test } from "bun:test";
import { Accounts } from "../src/accounts.ts";
import { AgentLoginError, proveAgentIdentity } from "../src/agent.ts";
import { createApp, type OAuthPort } from "../src/app.ts";
import { Store } from "../src/db.ts";
import { GAME_PASSWORD } from "../src/password.ts";
import { RateLimiter } from "../src/ratelimit.ts";
import { addIdentity, ALICE_DID, BOB_DID, depsFor, fakeNet, FakeRealm } from "./fakes.ts";

const APP_PASSWORD = "aaaa-bbbb-cccc-dddd";

describe("proveAgentIdentity", () => {
  test("app password on the resolved PDS proves the DID; the session is revoked", async () => {
    const net = fakeNet();
    addIdentity(net, { did: ALICE_DID, handle: "alice.bsky.social" });
    const proven = await proveAgentIdentity("@alice.bsky.social", APP_PASSWORD, depsFor(net));
    expect(proven).toEqual({ did: ALICE_DID, handle: "alice.bsky.social" });
    await Bun.sleep(0);
    const paths = net.requests.map((r) => `${r.method} ${new URL(r.url).pathname}`);
    expect(paths).toContain("POST /xrpc/com.atproto.server.createSession");
    expect(paths).toContain("POST /xrpc/com.atproto.server.deleteSession");
  });

  test("wrong app password -> 401", async () => {
    const net = fakeNet();
    addIdentity(net, { did: ALICE_DID, handle: "alice.bsky.social" });
    const err = await proveAgentIdentity("alice.bsky.social", "wrong", depsFor(net)).catch((e) => e);
    expect(err).toBeInstanceOf(AgentLoginError);
    expect((err as AgentLoginError).status).toBe(401);
  });

  test("a PDS that signs in a different DID -> 403", async () => {
    const net = fakeNet();
    addIdentity(net, { did: ALICE_DID, handle: "alice.bsky.social", sessionDid: BOB_DID });
    const err = (await proveAgentIdentity("alice.bsky.social", APP_PASSWORD, depsFor(net)).catch((e) => e)) as AgentLoginError;
    expect(err.status).toBe(403);
  });

  test("the main account password is refused (scope com.atproto.access)", async () => {
    const net = fakeNet();
    addIdentity(net, { did: ALICE_DID, handle: "alice.bsky.social", scope: "com.atproto.access" });
    const err = (await proveAgentIdentity("alice.bsky.social", APP_PASSWORD, depsFor(net)).catch((e) => e)) as AgentLoginError;
    expect(err.status).toBe(401);
    expect(err.message).toMatch(/app password/);
  });

  test("bad input never reaches the network", async () => {
    const net = fakeNet();
    for (const [h, p] of [
      [undefined, APP_PASSWORD],
      ["alice", APP_PASSWORD],
      ["alice.bsky.social", ""],
      ["alice.bsky.social", "x".repeat(300)],
    ] as const) {
      const err = (await proveAgentIdentity(h, p, depsFor(net)).catch((e) => e)) as AgentLoginError;
      expect(err.status).toBe(400);
    }
    expect(net.requests.length).toBe(0);
  });

  test("an unresolvable handle -> 502", async () => {
    const err = (await proveAgentIdentity("nobody.example.com", APP_PASSWORD, depsFor(fakeNet())).catch((e) => e)) as AgentLoginError;
    expect(err.status).toBe(502);
  });
});

const noOAuth: OAuthPort = {
  clientMetadata: {},
  jwks: { keys: [] },
  authorize: async () => new URL("https://auth.example.com/authorize"),
  callback: async () => {
    throw new Error("unused");
  },
};

function app(opts: { perIp?: number; perDid?: number } = {}) {
  const net = fakeNet();
  addIdentity(net, { did: ALICE_DID, handle: "alice.bsky.social" });
  const realm = new FakeRealm();
  const accounts = new Accounts({ store: new Store(":memory:"), soap: realm.run, wait: async () => {}, log: () => {} });
  const handle = createApp({
    accounts,
    identity: depsFor(net),
    oauth: noOAuth,
    scope: "atproto",
    realmAddress: "wrathplus.artivus.com.au:3724",
    loginsPerIp: new RateLimiter({ max: opts.perIp ?? 20, windowMs: 60_000 }),
    issuesPerDid: new RateLimiter({ max: opts.perDid ?? 10, windowMs: 60_000 }),
    log: () => {},
  });
  const login = (body: unknown, ip = "198.51.100.7") =>
    handle(
      new Request("https://wrathplus.artivus.com.au/api/agent/login", {
        method: "POST",
        headers: { "content-type": "application/json", "x-forwarded-for": ip },
        body: JSON.stringify(body),
      }),
      "127.0.0.1",
    );
  return { handle, login, realm, net };
}

describe("POST /api/agent/login", () => {
  test("returns account, password and realm; the realm has the account", async () => {
    const { login, realm } = app();
    const res = await login({ handle: "alice.bsky.social", app_password: APP_PASSWORD });
    expect(res.status).toBe(200);
    expect(res.headers.get("cache-control")).toBe("no-store");
    const body = (await res.json()) as { account: string; password: string; realm: string };
    expect(body.account).toBe("ALICE");
    expect(body.password).toMatch(GAME_PASSWORD);
    expect(body.realm).toBe("wrathplus.artivus.com.au:3724");
    expect(realm.accounts.get("ALICE")!.password).toBe(body.password);
  });

  test("the app password is never sent to the realm", async () => {
    const { login, realm } = app();
    await login({ handle: "alice.bsky.social", app_password: APP_PASSWORD });
    expect(realm.commands.some((c) => c.includes(APP_PASSWORD))).toBe(false);
  });

  test("wrong password -> 401 and no account", async () => {
    const { login, realm } = app();
    const res = await login({ handle: "alice.bsky.social", app_password: "nope" });
    expect(res.status).toBe(401);
    expect(realm.accounts.size).toBe(0);
  });

  test("rate limited per IP before any lookup", async () => {
    const { login, net } = app({ perIp: 2 });
    await login({ handle: "alice.bsky.social", app_password: "nope" });
    await login({ handle: "alice.bsky.social", app_password: "nope" });
    const before = net.requests.length;
    const res = await login({ handle: "alice.bsky.social", app_password: APP_PASSWORD });
    expect(res.status).toBe(429);
    expect(Number(res.headers.get("retry-after"))).toBeGreaterThan(0);
    expect(net.requests.length).toBe(before);
    // Another address is unaffected.
    expect((await login({ handle: "alice.bsky.social", app_password: APP_PASSWORD }, "198.51.100.8")).status).toBe(200);
  });

  test("rate limited per DID across addresses", async () => {
    const { login } = app({ perDid: 1 });
    expect((await login({ handle: "alice.bsky.social", app_password: APP_PASSWORD }, "198.51.100.1")).status).toBe(200);
    expect((await login({ handle: "alice.bsky.social", app_password: APP_PASSWORD }, "198.51.100.2")).status).toBe(429);
  });

  test("requires JSON", async () => {
    const { handle } = app();
    const res = await handle(new Request("https://x/api/agent/login", { method: "POST", body: "handle=a" }));
    expect(res.status).toBe(415);
  });
});

describe("other routes", () => {
  test("home page and unknown routes", async () => {
    const { handle } = app();
    const home = await handle(new Request("https://x/"));
    expect(home.status).toBe(200);
    expect(home.headers.get("content-security-policy")).toContain("default-src 'none'");
    expect((await handle(new Request("https://x/nope"))).status).toBe(404);
  });

  test("OAuth login redirects to the authorization server", async () => {
    const { handle } = app();
    const res = await handle(new Request("https://x/oauth/login", { method: "POST", body: new URLSearchParams({ handle: "alice.bsky.social" }) }));
    expect(res.status).toBe(303);
    expect(res.headers.get("location")).toBe("https://auth.example.com/authorize");
    const bad = await handle(new Request("https://x/oauth/login", { method: "POST", body: new URLSearchParams({ handle: "<script>" }) }));
    expect(bad.status).toBe(400);
  });
});
