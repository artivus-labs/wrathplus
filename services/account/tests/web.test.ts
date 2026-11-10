import { describe, expect, test } from "bun:test";
import { Store } from "../src/db.ts";
import { SESSION_COOKIE, WebSessions } from "../src/sessions.ts";
import { ALICE_DID, TEST_KEY } from "./fakes.ts";
import { harness } from "./harness.ts";

describe("web session cookie", () => {
  test("set after OAuth: signed, HttpOnly, Secure, SameSite=Lax, __Host- prefixed", async () => {
    const { signIn } = harness();
    const { res, setCookie } = await signIn();
    expect(res.status).toBe(303);
    expect(res.headers.get("location")).toBe("/");
    expect(setCookie.startsWith(`${SESSION_COOKIE}=`)).toBe(true);
    for (const flag of ["HttpOnly", "Secure", "SameSite=Lax", "Path=/"]) expect(setCookie).toContain(flag);
    expect(setCookie).not.toContain("Domain");
  });

  test("a forged or tampered cookie is no session", () => {
    const store = new Store(":memory:");
    const sessions = new WebSessions(store, TEST_KEY);
    const { setCookie } = sessions.create(ALICE_DID, "alice.bsky.social");
    const cookie = setCookie.split(";")[0]!;
    expect(sessions.get(cookie)?.did).toBe(ALICE_DID);
    const [name, value] = cookie.split("=") as [string, string];
    const [id, sig] = value.split(".") as [string, string];
    expect(sessions.get(`${name}=${id}x.${sig}`)).toBeUndefined();
    expect(sessions.get(`${name}=${id}.${sig.slice(0, -2)}AA`)).toBeUndefined();
    expect(new WebSessions(store, Buffer.alloc(32, 1)).get(cookie)).toBeUndefined();
    expect(sessions.get(null)).toBeUndefined();
  });

  test("sessions expire", () => {
    let now = 0;
    const sessions = new WebSessions(new Store(":memory:"), TEST_KEY, () => now, 1000);
    const cookie = sessions.create(ALICE_DID, "alice.bsky.social").setCookie.split(";")[0]!;
    now = 999;
    expect(sessions.get(cookie)).toBeDefined();
    now = 1001;
    expect(sessions.get(cookie)).toBeUndefined();
  });

  test("the database holds no usable cookie (ids are hashed)", async () => {
    const { signIn, store } = harness();
    const { cookie } = await signIn();
    const id = cookie.split("=")[1]!.split(".")[0]!;
    const rows = store.db.query<{ id_hash: string }, []>("SELECT id_hash FROM web_session").all();
    expect(rows.length).toBe(1);
    expect(rows[0]!.id_hash).not.toBe(id);
  });
});

describe("account page", () => {
  test("signed out: the sign-in form", async () => {
    const { get } = harness();
    const res = await get("/");
    expect(await res.text()).toContain('action="/oauth/login"');
    expect(res.headers.get("content-security-policy")).toContain("default-src 'none'");
  });

  test("signed in: account and the stored password, unchanged between views", async () => {
    const { signIn, get, realm } = harness();
    const { cookie } = await signIn();
    const page1 = await (await get("/", cookie)).text();
    const page2 = await (await get("/", cookie)).text();
    const password = realm.accounts.get("ALICE")!.password;
    expect(page1).toContain("ALICE");
    expect(page1).toContain(password);
    expect(page2).toContain(password);
    expect(realm.commands.filter((c) => c.startsWith(".account set password")).length).toBe(0);
  });

  test("reset needs the session's CSRF token and a same-origin request", async () => {
    const { signIn, get, post, realm, csrfOf } = harness();
    const { cookie } = await signIn();
    const csrf = await csrfOf(await get("/", cookie));
    const before = realm.accounts.get("ALICE")!.password;
    expect((await post("/account/reset", { csrf: "wrong" }, cookie)).status).toBe(403);
    expect((await post("/account/reset", {}, cookie)).status).toBe(403);
    expect((await post("/account/reset", { csrf }, cookie, "https://evil.example")).status).toBe(403);
    expect((await post("/account/reset", { csrf })).status).toBe(303); // no session: back home
    expect(realm.accounts.get("ALICE")!.password).toBe(before);
    const ok = await post("/account/reset", { csrf }, cookie);
    expect(ok.status).toBe(200);
    const after = realm.accounts.get("ALICE")!.password;
    expect(after).not.toBe(before);
    expect(await ok.text()).toContain(after);
    expect(await (await get("/", cookie)).text()).toContain(after);
  });

  test("reset is rate limited per DID", async () => {
    const { signIn, get, post, csrfOf } = harness({ perDid: 1 });
    const { cookie } = await signIn();
    const csrf = await csrfOf(await get("/", cookie));
    expect((await post("/account/reset", { csrf }, cookie)).status).toBe(200);
    expect((await post("/account/reset", { csrf }, cookie)).status).toBe(429);
  });

  test("sign out ends the session server-side", async () => {
    const { signIn, get, post, csrfOf } = harness();
    const { cookie } = await signIn();
    const csrf = await csrfOf(await get("/", cookie));
    const res = await post("/logout", { csrf }, cookie);
    expect(res.status).toBe(303);
    expect(res.headers.get("set-cookie")).toContain("Max-Age=0");
    expect(await (await get("/", cookie)).text()).toContain('action="/oauth/login"');
  });

  test("OAuth: a cancelled sign-in and an unverifiable handle set no cookie", async () => {
    const { get, net } = harness();
    const cancelled = await get("/oauth/callback?error=access_denied");
    expect(cancelled.status).toBe(400);
    expect(cancelled.headers.get("set-cookie")).toBeNull();
    net.txt["_atproto.alice.bsky.social"] = ["did=did:plc:bbbbbbbbbbbbbbbbbbbbbbbb"];
    const unverified = await get(`/oauth/callback?did=${ALICE_DID}`);
    expect(unverified.status).toBe(400);
    expect(unverified.headers.get("set-cookie")).toBeNull();
  });

  test("OAuth login redirects to the authorization server; bad handles are refused", async () => {
    const { post } = harness();
    const res = await post("/oauth/login", { handle: "alice.bsky.social" });
    expect(res.status).toBe(303);
    expect(res.headers.get("location")).toBe("https://auth.example.com/authorize");
    expect((await post("/oauth/login", { handle: "<script>" })).status).toBe(400);
  });

  test("unknown routes", async () => {
    expect((await harness().get("/nope")).status).toBe(404);
  });
});

describe("what is published", () => {
  const guide = "https://github.com/artivus-labs/wrathplus/blob/main/docs/BETA.md";

  test("the sign-in page and the account page say what is published, with a link to the beta guide", async () => {
    const { signIn, get } = harness();
    const signedOut = await (await get("/")).text();
    const { cookie } = await signIn();
    const signedIn = await (await get("/", cookie)).text();
    for (const page of [signedOut, signedIn]) {
      expect(page).toContain("level and achievement milestones as public records in your own ATProto repository");
      expect(page).toContain(`href="${guide}#what-is-published"`);
    }
    expect(signedOut).toContain(`href="${guide}#agents"`);
    expect(signedIn).toContain(`href="${guide}"`);
  });

  test("the client sign-in page carries it too", async () => {
    const { get } = harness();
    const page = await (await get("/client/login?redirect=http%3A%2F%2F127.0.0.1%3A5000%2Fcallback&state=0123456789abcdef0123&code_challenge=" + "a".repeat(43))).text();
    expect(page).toContain('action="/oauth/login"');
    expect(page).toContain(`href="${guide}#what-is-published"`);
  });
});

describe("copy buttons", () => {
  test("the account page has a copy button for the account, password and realm, from /copy.js only", async () => {
    const { signIn, get } = harness();
    const { cookie } = await signIn();
    const res = await get("/", cookie);
    const page = await res.text();
    for (const id of ["account", "password", "realm"]) {
      expect(page).toContain(`id="${id}"`);
      expect(page).toContain(`data-copy="${id}"`);
    }
    expect(page).toContain('<script src="/copy.js" defer></script>');
    // No inline script: every <script> has a src.
    expect(page.match(/<script(?![^>]*\bsrc=)[^>]*>/g)).toBeNull();
    const csp = res.headers.get("content-security-policy") ?? "";
    expect(csp).toContain("script-src 'self'");
    expect(csp.split(";").find((d) => d.trim().startsWith("script-src"))).not.toContain("unsafe-inline");
  });

  test("/copy.js is served as JavaScript and copies the target's text", async () => {
    const { get } = harness();
    const res = await get("/copy.js");
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toContain("text/javascript");
    const js = await res.text();
    expect(js).toContain("navigator.clipboard.writeText");
    expect(js).toContain("data-copy");
  });
});
