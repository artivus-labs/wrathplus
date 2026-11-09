import { describe, expect, test } from "bun:test";
import { Accounts } from "../src/accounts.ts";
import { Store } from "../src/db.ts";
import { GAME_PASSWORD } from "../src/password.ts";
import { ALICE_DID, BOB_DID, FakeRealm } from "./fakes.ts";

function setup(realm = new FakeRealm()) {
  const store = new Store(":memory:");
  const logs: string[] = [];
  const accounts = new Accounts({ store, soap: realm.run, wait: async () => {}, log: (m) => logs.push(m) });
  return { realm, store, accounts, logs };
}

describe("Accounts.issue", () => {
  test("creates, enables Wrath, links in the realm, stores the mapping", async () => {
    const { realm, store, accounts } = setup();
    const issued = await accounts.issue(ALICE_DID, "alice.bsky.social");
    expect(issued.account).toBe("ALICE");
    expect(issued.created).toBe(true);
    expect(issued.password).toMatch(GAME_PASSWORD);
    expect(realm.accounts.get("ALICE")).toEqual({ password: issued.password, addon: 2 });
    expect(realm.links.get("ALICE")).toEqual({ did: ALICE_DID, handle: "alice.bsky.social" });
    expect(store.linkByDid(ALICE_DID)?.account).toBe("ALICE");
  });

  test("one account per DID: a second sign-in resets the password on the same account", async () => {
    const { realm, store, accounts } = setup();
    const first = await accounts.issue(ALICE_DID, "alice.bsky.social");
    const second = await accounts.issue(ALICE_DID, "alice-renamed.bsky.social");
    expect(second.account).toBe(first.account);
    expect(second.created).toBe(false);
    expect(second.password).not.toBe(first.password);
    expect(realm.accounts.get("ALICE")!.password).toBe(second.password);
    expect(realm.accounts.size).toBe(1);
    expect(store.linkByDid(ALICE_DID)?.handle).toBe("alice-renamed.bsky.social");
    expect(realm.links.get("ALICE")?.handle).toBe("alice-renamed.bsky.social");
  });

  test("never takes over an account it did not create (bots, admins)", async () => {
    const realm = new FakeRealm();
    realm.accounts.set("ALICE", { password: "OWNERS", addon: 2 });
    const { accounts } = setup(realm);
    const issued = await accounts.issue(ALICE_DID, "alice.bsky.social");
    expect(issued.account).not.toBe("ALICE");
    expect(issued.account.startsWith("ALICE")).toBe(true);
    expect(realm.accounts.get("ALICE")!.password).toBe("OWNERS");
  });

  test("same cleaned handle, different DIDs, one realm", async () => {
    const { accounts } = setup();
    const a = await accounts.issue(ALICE_DID, "alice.bsky.social");
    const b = await accounts.issue(BOB_DID, "alice.bsky.social");
    expect(a.account).toBe("ALICE");
    expect(b.account).not.toBe("ALICE");
    expect(b.account).toMatch(/^ALICE[A-Z2-7]{5}$/);
  });

  test("concurrent first sign-ins never share a name", async () => {
    const { accounts } = setup();
    const [a, b] = await Promise.all([accounts.issue(ALICE_DID, "alice.bsky.social"), accounts.issue(BOB_DID, "alice.bsky.social")]);
    expect(a.account).not.toBe(b.account);
  });

  test("retries while the realm is still creating the account", async () => {
    const realm = new FakeRealm();
    realm.lag = 3;
    const { accounts } = setup(realm);
    await accounts.issue(ALICE_DID, "alice.bsky.social");
    expect(realm.accounts.get("ALICE")!.addon).toBe(2);
    expect(realm.commands.filter((c) => c.startsWith(".account set addon")).length).toBe(4);
  });

  test("works without the realm module (link failure is logged, not fatal)", async () => {
    const realm = new FakeRealm();
    realm.moduleInstalled = false;
    const { accounts, logs } = setup(realm);
    const issued = await accounts.issue(ALICE_DID, "alice.bsky.social");
    expect(issued.account).toBe("ALICE");
    expect(logs.some((l) => l.includes("realm link"))).toBe(true);
  });

  test("recreates a linked account the realm lost", async () => {
    const { realm, accounts } = setup();
    await accounts.issue(ALICE_DID, "alice.bsky.social");
    realm.accounts.delete("ALICE");
    const again = await accounts.issue(ALICE_DID, "alice.bsky.social");
    expect(again.account).toBe("ALICE");
    expect(again.created).toBe(true);
    expect(realm.accounts.get("ALICE")!.password).toBe(again.password);
  });

  test("errors never carry the password", async () => {
    const realm = new FakeRealm();
    const pw = "ABCDEFGHJKLMNPQR";
    const failing = async (c: string) => {
      if (c.startsWith(".account create")) throw new Error(`boom ${c}`);
      return realm.run(c);
    };
    const accounts = new Accounts({ store: new Store(":memory:"), soap: failing, password: () => pw, wait: async () => {}, log: () => {} });
    const err = await accounts.issue(ALICE_DID, "alice.bsky.social").catch((e: Error) => e);
    expect((err as Error).message).not.toContain(pw);
  });
});
