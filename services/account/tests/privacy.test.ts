import { describe, expect, test } from "bun:test";
import { ALICE_DID } from "./fakes.ts";
import { harness } from "./harness.ts";

async function begin(h: ReturnType<typeof harness>, publish = false) {
  await h.post("/oauth/login", { handle: "alice.bsky.social", ...(publish ? { publish: "yes" } : {}) });
  return h.authorized.at(-1)!.state!;
}

describe("private registration", () => {
  test("an old opt-in callback cannot destroy a newer successful grant", async () => {
    const h = harness();
    const old = await begin(h, true);
    const fresh = await begin(h, true);
    expect((await h.signIn(ALICE_DID, fresh)).res.status).toBe(303);
    expect((await h.signIn(ALICE_DID, old)).res.status).toBe(400);
    expect(h.store.publishingEnabled(ALICE_DID)).toBe(true);
    expect(h.store.hasSession(ALICE_DID)).toBe(true);
  });

  test("registration defaults unchecked; identity-only OAuth discards its credential", async () => {
    const h = harness();
    const page = await (await h.get("/")).text();
    expect(page).toContain('name="publish" value="yes"');
    expect(page).not.toMatch(/type="checkbox"[^>]*checked/);
    const state = await begin(h);
    expect(h.authorized[0]!.scope).toBe("atproto");
    const { cookie } = await h.signIn(ALICE_DID, state);
    expect(h.store.publishingEnabled(ALICE_DID)).toBe(false);
    expect(h.store.hasSession(ALICE_DID)).toBe(false);
    expect(h.oauthState.signOuts).toBe(1);
    expect(await (await h.get("/", cookie)).text()).toContain("Public records: disabled");
    expect(h.realm.accounts.has("ALICE")).toBe(true);
  });

  test("opt-in requests scoped writes and retains session only with matching grant", async () => {
    const h = harness();
    const state = await begin(h, true);
    expect(h.authorized[0]!.scope).toContain("repo:au.com.artivus.wrathplus.character?");
    const { cookie } = await h.signIn(ALICE_DID, state);
    expect(h.store.publishingEnabled(ALICE_DID)).toBe(true);
    expect(h.store.hasSession(ALICE_DID)).toBe(true);
    expect(await (await h.get("/", cookie)).text()).toContain("Stop public publishing");
  });

  test("a reduced grant cannot enable publishing", async () => {
    const h = harness();
    const state = await begin(h, true);
    h.oauthState.scope = "atproto";
    await h.signIn(ALICE_DID, state);
    expect(h.store.publishingEnabled(ALICE_DID)).toBe(false);
    expect(h.store.hasSession(ALICE_DID)).toBe(false);
  });

  test("withdrawal requires CSRF and same origin; stale opt-in cannot undo it", async () => {
    const h = harness();
    const state = await begin(h, true);
    const { cookie } = await h.signIn(ALICE_DID, state);
    const oldState = await begin(h, true);
    const csrf = await h.csrfOf(await h.get("/", cookie));
    expect((await h.post("/account/privacy", { csrf: "bad" }, cookie)).status).toBe(403);
    expect((await h.post("/account/privacy", { csrf }, cookie, "https://evil.example")).status).toBe(403);
    expect(h.store.publishingEnabled(ALICE_DID)).toBe(true);
    expect((await h.post("/account/privacy", { csrf }, cookie)).status).toBe(303);
    expect(h.store.hasSession(ALICE_DID)).toBe(false);
    await h.signIn(ALICE_DID, oldState);
    expect(h.store.publishingEnabled(ALICE_DID)).toBe(false);
    expect(h.store.hasSession(ALICE_DID)).toBe(false);
    expect(h.realm.accounts.size).toBe(1);
  });

  test("ordinary sign-in withdraws previously enabled publishing without changing game password", async () => {
    const h = harness();
    await h.signIn(ALICE_DID, await begin(h, true));
    const password = h.realm.accounts.get("ALICE")!.password;
    await h.signIn(ALICE_DID, await begin(h));
    expect(h.store.publishingEnabled(ALICE_DID)).toBe(false);
    expect(h.store.hasSession(ALICE_DID)).toBe(false);
    expect(h.realm.accounts.get("ALICE")!.password).toBe(password);
  });

  test("legacy callback and wrong-DID consent never grant publishing", async () => {
    const h = harness();
    await h.signIn();
    expect(h.store.publishingEnabled(ALICE_DID)).toBe(false);
    const state = JSON.stringify({ v: 1, did: "did:plc:other", version: 0, publish: true });
    await h.signIn(ALICE_DID, state);
    expect(h.store.publishingEnabled(ALICE_DID)).toBe(false);
    expect(h.store.hasSession(ALICE_DID)).toBe(false);
  });

  test("OAuth login refuses cross-origin consent submissions", async () => {
    const h = harness();
    expect((await h.post("/oauth/login", { handle: "alice.bsky.social", publish: "yes" }, undefined, "https://evil.example")).status).toBe(403);
    expect(h.authorized).toHaveLength(0);
  });
});
