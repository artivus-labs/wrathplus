import { describe, expect, test } from "bun:test";
import { Accounts } from "../src/accounts.ts";
import { Store } from "../src/db.ts";
import { GAME_PASSWORD } from "../src/password.ts";
import { PasswordCipher } from "../src/secrets.ts";
import { ALICE_DID, BOB_DID, FakeRealm, TEST_KEY } from "./fakes.ts";

function setup(realm = new FakeRealm(), store = new Store(":memory:")) {
  const logs: string[] = [];
  const cipher = new PasswordCipher(TEST_KEY);
  const accounts = new Accounts({ store, soap: realm.run, cipher, wait: async () => {}, log: (m) => logs.push(m) });
  return { realm, store, accounts, logs, cipher };
}

describe("Accounts.ensure", () => {
  test("first time: creates, enables Wrath, links in the realm, stores the password encrypted", async () => {
    const { realm, store, accounts } = setup();
    const a = await accounts.ensure(ALICE_DID, "alice.bsky.social");
    expect(a).toMatchObject({ account: "ALICE", created: true, passwordChanged: true });
    expect(a.password).toMatch(GAME_PASSWORD);
    expect(realm.accounts.get("ALICE")).toEqual({ password: a.password, addon: 2 });
    expect(realm.links.get("ALICE")).toEqual({ did: ALICE_DID, handle: "alice.bsky.social" });
    const link = store.linkByDid(ALICE_DID)!;
    expect(link.account).toBe("ALICE");
    expect(link.passwordEnc).toStartWith("v1:");
    expect(link.passwordEnc).not.toContain(a.password);
  });

  test("afterwards: the same account and the same password, no realm password change", async () => {
    const { realm, accounts } = setup();
    const first = await accounts.ensure(ALICE_DID, "alice.bsky.social");
    const before = realm.commands.length;
    const again = await accounts.ensure(ALICE_DID, "alice.bsky.social");
    expect(again).toMatchObject({ account: first.account, password: first.password, created: false, passwordChanged: false });
    const sent = realm.commands.slice(before);
    expect(sent.every((c) => c.startsWith(".atproto link"))).toBe(true);
  });

  test("a new handle is stored and passed to the realm, the password kept", async () => {
    const { realm, store, accounts } = setup();
    const first = await accounts.ensure(ALICE_DID, "alice.bsky.social");
    const again = await accounts.ensure(ALICE_DID, "alice2.bsky.social");
    expect(again.password).toBe(first.password);
    expect(store.linkByDid(ALICE_DID)!.handle).toBe("alice2.bsky.social");
    expect(realm.links.get("ALICE")!.handle).toBe("alice2.bsky.social");
  });

  test("a link from v1 (no stored password) or under a lost key gets a new password", async () => {
    const realm = new FakeRealm();
    const store = new Store(":memory:");
    const { accounts } = setup(realm, store);
    const first = await accounts.ensure(ALICE_DID, "alice.bsky.social");
    store.db.query("UPDATE account_link SET password_enc = NULL").run();
    const v1 = await accounts.ensure(ALICE_DID, "alice.bsky.social");
    expect(v1.passwordChanged).toBe(true);
    expect(v1.password).not.toBe(first.password);
    expect(realm.accounts.get("ALICE")!.password).toBe(v1.password);
    const otherKey = new Accounts({ store, soap: realm.run, cipher: new PasswordCipher(Buffer.alloc(32, 9)), wait: async () => {}, log: () => {} });
    const lost = await otherKey.ensure(ALICE_DID, "alice.bsky.social");
    expect(lost.passwordChanged).toBe(true);
    expect(realm.accounts.get("ALICE")!.password).toBe(lost.password);
  });

  test("never takes over an account it did not create (bots, admins)", async () => {
    const realm = new FakeRealm();
    realm.accounts.set("ALICE", { password: "OWNERS", addon: 2 });
    const { accounts } = setup(realm);
    const a = await accounts.ensure(ALICE_DID, "alice.bsky.social");
    expect(a.account).toMatch(/^ALICE[A-Z2-7]{5}$/);
    expect(realm.accounts.get("ALICE")!.password).toBe("OWNERS");
  });

  test("same cleaned handle, different DIDs: distinct, stable names", async () => {
    const { accounts } = setup();
    const a = await accounts.ensure(ALICE_DID, "alice.bsky.social");
    const b = await accounts.ensure(BOB_DID, "alice.bsky.social");
    expect(a.account).toBe("ALICE");
    expect(b.account).toMatch(/^ALICE[A-Z2-7]{5}$/);
    expect((await accounts.ensure(BOB_DID, "alice.bsky.social")).account).toBe(b.account);
  });

  test("concurrent first sign-ins never share a name", async () => {
    const { accounts } = setup();
    const [a, b] = await Promise.all([accounts.ensure(ALICE_DID, "alice.bsky.social"), accounts.ensure(BOB_DID, "alice.bsky.social")]);
    expect(a.account).not.toBe(b.account);
  });

  test("retries while the realm is still creating the account", async () => {
    const realm = new FakeRealm();
    realm.lag = 3;
    const { accounts } = setup(realm);
    await accounts.ensure(ALICE_DID, "alice.bsky.social");
    expect(realm.accounts.get("ALICE")!.addon).toBe(2);
    expect(realm.commands.filter((c) => c.startsWith(".account set addon")).length).toBe(4);
  });

  test("works without the realm module (link failure is logged, not fatal)", async () => {
    const realm = new FakeRealm();
    realm.moduleInstalled = false;
    const { accounts, logs } = setup(realm);
    expect((await accounts.ensure(ALICE_DID, "alice.bsky.social")).account).toBe("ALICE");
    expect(logs.some((l) => l.includes("realm link"))).toBe(true);
  });
});

describe("Accounts.reset", () => {
  test("sets and stores a new password on the same account", async () => {
    const { realm, accounts } = setup();
    const first = await accounts.ensure(ALICE_DID, "alice.bsky.social");
    const reset = await accounts.reset(ALICE_DID, "alice.bsky.social");
    expect(reset).toMatchObject({ account: first.account, created: false, passwordChanged: true });
    expect(reset.password).not.toBe(first.password);
    expect(realm.accounts.get("ALICE")!.password).toBe(reset.password);
    expect((await accounts.ensure(ALICE_DID, "alice.bsky.social")).password).toBe(reset.password);
    expect(realm.accounts.size).toBe(1);
  });

  test("recreates a linked account the realm lost", async () => {
    const { realm, accounts } = setup();
    await accounts.ensure(ALICE_DID, "alice.bsky.social");
    realm.accounts.delete("ALICE");
    const again = await accounts.reset(ALICE_DID, "alice.bsky.social");
    expect(again).toMatchObject({ account: "ALICE", created: true });
    expect(realm.accounts.get("ALICE")).toEqual({ password: again.password, addon: 2 });
  });

  test("errors never carry the password", async () => {
    const realm = new FakeRealm();
    const pw = "ABCDEFGHJKLMNPQR";
    const failing = async (c: string) => {
      if (c.startsWith(".account create")) throw new Error(`boom ${c}`);
      return realm.run(c);
    };
    const accounts = new Accounts({ store: new Store(":memory:"), soap: failing, cipher: new PasswordCipher(TEST_KEY), password: () => pw, wait: async () => {}, log: () => {} });
    const err = await accounts.reset(ALICE_DID, "alice.bsky.social").catch((e: Error) => e);
    expect((err as Error).message).not.toContain(pw);
  });
});
