import { describe, expect, test } from "bun:test";
import { checkClientRequest, checkRedirect, ClientCodes, CODE_TTL_MS, s256 } from "../src/client.ts";
import { Store } from "../src/db.ts";
import { ALICE_DID } from "./fakes.ts";
import { harness, ORIGIN } from "./harness.ts";

const VERIFIER = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"; // RFC 7636 appendix B
const STATE = "af0ifjsldkj-state-0123";

const params = (over: Record<string, string> = {}) =>
  new URLSearchParams({ redirect: "http://127.0.0.1:53682/callback", state: STATE, code_challenge: CHALLENGE, ...over });

describe("redirect validation", () => {
  test("loopback IP literals with any explicit port", () => {
    expect(checkRedirect("http://127.0.0.1:53682/callback")).toBe("http://127.0.0.1:53682/callback");
    expect(checkRedirect("http://[::1]:1/callback")).toBe("http://[::1]:1/callback");
    expect(checkRedirect("http://127.0.0.1:65535/")).toBe("http://127.0.0.1:65535/");
  });
  test("everything else is refused", () => {
    for (const bad of [
      "http://localhost:5000/callback",
      "https://127.0.0.1:5000/callback",
      "http://127.0.0.1/callback",
      "http://127.0.0.2:5000/callback",
      "http://10.0.0.1:5000/callback",
      "http://evil.com:5000/callback",
      "http://127.0.0.1.evil.com:5000/callback",
      "http://user:pw@127.0.0.1:5000/callback",
      "http://127.0.0.1:5000/callback?next=https://evil.com",
      "http://127.0.0.1:5000/callback#x",
      "http://127.0.0.1:99999/callback",
      "javascript:alert(1)",
      "//127.0.0.1:5000/callback",
      "",
    ]) {
      expect(checkRedirect(bad)).toBeUndefined();
    }
  });
});

describe("request validation", () => {
  test("a well-formed request", () => {
    expect(checkClientRequest(params())).toEqual({ ok: true, request: { redirect: "http://127.0.0.1:53682/callback", state: STATE, challenge: CHALLENGE } });
  });
  test("state, challenge and method are checked", () => {
    expect(checkClientRequest(params({ state: "short" })).ok).toBe(false);
    expect(checkClientRequest(params({ state: `${STATE}<script>` })).ok).toBe(false);
    expect(checkClientRequest(params({ code_challenge: "abc" })).ok).toBe(false);
    expect(checkClientRequest(params({ code_challenge_method: "plain" })).ok).toBe(false);
    expect(checkClientRequest(params({ code_challenge_method: "S256" })).ok).toBe(true);
  });
  test("S256 matches RFC 7636's example", () => {
    expect(s256(VERIFIER)).toBe(CHALLENGE);
  });
});

describe("codes", () => {
  const request = { redirect: "http://127.0.0.1:53682/callback", state: STATE, challenge: CHALLENGE };
  test("single use", () => {
    const codes = new ClientCodes(new Store(":memory:"));
    const code = codes.issue(ALICE_DID, "alice.bsky.social", request);
    expect(codes.redeem(code, VERIFIER)).toEqual({ ok: true, did: ALICE_DID, handle: "alice.bsky.social" });
    expect(codes.redeem(code, VERIFIER)).toEqual({ ok: false, error: "invalid_grant" });
  });
  test("PKCE: a wrong verifier fails and burns the code", () => {
    const codes = new ClientCodes(new Store(":memory:"));
    const code = codes.issue(ALICE_DID, "alice.bsky.social", request);
    expect(codes.redeem(code, "x".repeat(43))).toEqual({ ok: false, error: "invalid_grant" });
    expect(codes.redeem(code, VERIFIER)).toEqual({ ok: false, error: "invalid_grant" });
    expect(codes.redeem(code, "short")).toEqual({ ok: false, error: "invalid_request" });
  });
  test("expire after 60 seconds", () => {
    let now = 0;
    const codes = new ClientCodes(new Store(":memory:"), () => now);
    const a = codes.issue(ALICE_DID, "alice.bsky.social", request);
    const b = codes.issue(ALICE_DID, "alice.bsky.social", request);
    now = CODE_TTL_MS - 1;
    expect(codes.redeem(a, VERIFIER).ok).toBe(true);
    now = CODE_TTL_MS + 1;
    expect(codes.redeem(b, VERIFIER)).toEqual({ ok: false, error: "invalid_grant" });
  });
  test("codes are stored hashed", () => {
    const store = new Store(":memory:");
    const code = new ClientCodes(store).issue(ALICE_DID, "alice.bsky.social", request);
    const row = store.db.query<{ code_hash: string }, []>("SELECT code_hash FROM client_code").get();
    expect(row!.code_hash).not.toBe(code);
  });
});

describe("the flow through the app", () => {
  async function approve(h: ReturnType<typeof harness>, cookie: string, decision = "approve") {
    const page = await h.get(`/client/login?${params()}`, cookie);
    const text = await page.text();
    expect(text).toContain("Sign in WrathPlus client as <strong>@alice.bsky.social</strong>?");
    const csrf = /name="csrf" value="([^"]+)"/.exec(text)![1]!;
    const res = await h.post("/client/approve", { redirect: "http://127.0.0.1:53682/callback", state: STATE, code_challenge: CHALLENGE, code_challenge_method: "S256", csrf, decision }, cookie);
    const target = /http-equiv="refresh" content="0;url=([^"]+)"/.exec(await res.text())?.[1]?.replace(/&amp;/g, "&");
    return { res, target: target ? new URL(target) : undefined };
  }

  test("signed out: sign in first, then OAuth comes back to the confirmation", async () => {
    const h = harness();
    const page = await h.get(`/client/login?${params()}`);
    const text = await page.text();
    expect(text).toContain('action="/oauth/login"');
    const next = /name="next" value="([^"]+)"/.exec(text)![1]!.replace(/&amp;/g, "&");
    expect(next).toBe(`/client/login?${params()}`);
    await h.post("/oauth/login", { handle: "alice.bsky.social", next });
    expect(h.authorized[0]).toMatchObject({ handle: "alice.bsky.social", scope: "atproto" });
    expect(JSON.parse(h.authorized[0]!.state!).next).toBe(next);
    const { res } = await h.signIn(ALICE_DID, h.authorized[0]!.state);
    expect(res.headers.get("location")).toBe(next);
  });

  test("OAuth state cannot send people anywhere but the client sign-in page", async () => {
    const h = harness();
    await h.post("/oauth/login", { handle: "alice.bsky.social", next: "https://evil.com/" });
    expect(JSON.parse(h.authorized[0]!.state!).next).toBeUndefined();
    const { res } = await h.signIn(ALICE_DID, "//evil.com/");
    expect(res.headers.get("location")).toBe("/");
  });

  test("approve -> loopback with code, state and issuer -> token gives the stored account", async () => {
    const h = harness();
    const { cookie } = await h.signIn();
    const { target } = await approve(h, cookie);
    expect(`${target!.origin}${target!.pathname}`).toBe("http://127.0.0.1:53682/callback");
    expect(target!.searchParams.get("state")).toBe(STATE);
    expect(target!.searchParams.get("iss")).toBe(ORIGIN);
    const code = target!.searchParams.get("code")!;
    const token = await h.postJson("/client/token", { code, code_verifier: VERIFIER });
    expect(token.status).toBe(200);
    const body = (await token.json()) as { account: string; password: string; realm: string };
    expect(body).toMatchObject({ account: "ALICE", realm: "wrathplus.artivus.com.au:3724" });
    expect(body.password).toBe(h.realm.accounts.get("ALICE")!.password);
    // Single use through the API too.
    expect((await h.postJson("/client/token", { code, code_verifier: VERIFIER })).status).toBe(400);
  });

  test("the token endpoint rejects a wrong verifier and an expired code", async () => {
    const h = harness();
    const { cookie } = await h.signIn();
    const first = (await approve(h, cookie)).target!.searchParams.get("code")!;
    const wrong = await h.postJson("/client/token", { code: first, code_verifier: "y".repeat(43) });
    expect(wrong.status).toBe(400);
    expect(await wrong.json()).toEqual({ error: "invalid_grant" });
    const second = (await approve(h, cookie)).target!.searchParams.get("code")!;
    h.clock.now += CODE_TTL_MS + 1;
    expect((await h.postJson("/client/token", { code: second, code_verifier: VERIFIER })).status).toBe(400);
  });

  test("cancel returns access_denied with the state, and no code", async () => {
    const h = harness();
    const { cookie } = await h.signIn();
    const { target } = await approve(h, cookie, "deny");
    expect(target!.searchParams.get("error")).toBe("access_denied");
    expect(target!.searchParams.get("state")).toBe(STATE);
    expect(target!.searchParams.get("code")).toBeNull();
  });

  test("approval needs the CSRF token and a same-origin post", async () => {
    const h = harness();
    const { cookie } = await h.signIn();
    const fields = { redirect: "http://127.0.0.1:53682/callback", state: STATE, code_challenge: CHALLENGE, decision: "approve" };
    expect((await h.post("/client/approve", { ...fields, csrf: "nope" }, cookie)).status).toBe(403);
    const csrf = /name="csrf" value="([^"]+)"/.exec(await (await h.get(`/client/login?${params()}`, cookie)).text())![1]!;
    expect((await h.post("/client/approve", { ...fields, csrf }, cookie, "https://evil.example")).status).toBe(403);
  });

  test("an invalid request is an error page, never a redirect", async () => {
    const h = harness();
    const { cookie } = await h.signIn();
    for (const bad of [params({ redirect: "https://evil.com/cb" }), params({ state: "x" }), params({ code_challenge: "" })]) {
      const res = await h.get(`/client/login?${bad}`, cookie);
      expect(res.status).toBe(400);
      expect(res.headers.get("location")).toBeNull();
      expect(await res.text()).not.toContain("http-equiv");
    }
    const csrf = /name="csrf" value="([^"]+)"/.exec(await (await h.get(`/client/login?${params()}`, cookie)).text())![1]!;
    const tampered = await h.post("/client/approve", { redirect: "http://evil.com:80/", state: STATE, code_challenge: CHALLENGE, csrf, decision: "approve" }, cookie);
    expect(tampered.status).toBe(400);
  });

  test("token requests are rate limited per address", async () => {
    const h = harness({ perIp: 2 });
    await h.postJson("/client/token", { code: "a", code_verifier: VERIFIER });
    await h.postJson("/client/token", { code: "b", code_verifier: VERIFIER });
    expect((await h.postJson("/client/token", { code: "c", code_verifier: VERIFIER })).status).toBe(429);
  });
});
