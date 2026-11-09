import { describe, expect, test } from "bun:test";
import { AgentLoginError, proveAgentIdentity } from "../src/agent.ts";
import { GAME_PASSWORD } from "../src/password.ts";
import { addIdentity, ALICE_DID, BOB_DID, depsFor, fakeNet } from "./fakes.ts";
import { APP_PASSWORD, harness } from "./harness.ts";

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

describe("POST /api/agent/login", () => {
  const body = { handle: "alice.bsky.social", app_password: APP_PASSWORD };

  test("returns account, password and realm; the realm has the account", async () => {
    const { postJson, realm } = harness();
    const res = await postJson("/api/agent/login", body);
    expect(res.status).toBe(200);
    expect(res.headers.get("cache-control")).toBe("no-store");
    const out = (await res.json()) as { account: string; password: string; realm: string };
    expect(out.account).toBe("ALICE");
    expect(out.password).toMatch(GAME_PASSWORD);
    expect(out.realm).toBe("wrathplus.artivus.com.au:3724");
    expect(realm.accounts.get("ALICE")!.password).toBe(out.password);
  });

  test("signing in again returns the stored password; reset: true issues a new one", async () => {
    const { postJson, realm } = harness();
    const first = (await (await postJson("/api/agent/login", body)).json()) as { password: string };
    const again = (await (await postJson("/api/agent/login", body)).json()) as { password: string };
    expect(again.password).toBe(first.password);
    expect(realm.commands.filter((c) => c.startsWith(".account set password")).length).toBe(0);
    const reset = (await (await postJson("/api/agent/login", { ...body, reset: true })).json()) as { password: string };
    expect(reset.password).not.toBe(first.password);
    expect(realm.accounts.get("ALICE")!.password).toBe(reset.password);
    expect((await postJson("/api/agent/login", { ...body, reset: "yes" })).status).toBe(400);
  });

  test("the app password is never sent to the realm", async () => {
    const { postJson, realm } = harness();
    await postJson("/api/agent/login", body);
    expect(realm.commands.some((c) => c.includes(APP_PASSWORD))).toBe(false);
  });

  test("wrong password -> 401 and no account", async () => {
    const { postJson, realm } = harness();
    expect((await postJson("/api/agent/login", { ...body, app_password: "nope" })).status).toBe(401);
    expect(realm.accounts.size).toBe(0);
  });

  test("rate limited per IP before any lookup", async () => {
    const { postJson, net } = harness({ perIp: 2 });
    await postJson("/api/agent/login", { ...body, app_password: "nope" });
    await postJson("/api/agent/login", { ...body, app_password: "nope" });
    const before = net.requests.length;
    const res = await postJson("/api/agent/login", body);
    expect(res.status).toBe(429);
    expect(Number(res.headers.get("retry-after"))).toBeGreaterThan(0);
    expect(net.requests.length).toBe(before);
    expect((await postJson("/api/agent/login", body, "198.51.100.8")).status).toBe(200);
  });

  test("rate limited per DID across addresses", async () => {
    const { postJson } = harness({ perDid: 1 });
    expect((await postJson("/api/agent/login", body, "198.51.100.1")).status).toBe(200);
    expect((await postJson("/api/agent/login", body, "198.51.100.2")).status).toBe(429);
  });

  test("requires JSON", async () => {
    const { handle } = harness();
    expect((await handle(new Request("https://x/api/agent/login", { method: "POST", body: "handle=a" }))).status).toBe(415);
  });
});
