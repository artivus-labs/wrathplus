import { describe, expect, test } from "bun:test";
import { Database } from "bun:sqlite";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Store } from "../src/db.ts";
import { ALICE_DID } from "./fakes.ts";

describe("publication consent", () => {
  test("new accounts and unknown DIDs are private; schema defaults are explicitly zero", () => {
    const store = new Store(":memory:");
    try {
      expect(store.publishingEnabled(ALICE_DID)).toBe(false);
      expect(store.publishingVersion(ALICE_DID)).toBe(0);
      store.insertLink(ALICE_DID, "ALICE", "alice.test", "encrypted", 123);
      expect(store.publishingEnabled(ALICE_DID)).toBe(false);
      store.db.query("INSERT INTO publication_consent (did) VALUES (?)").run(ALICE_DID);
      expect(store.publishingEnabled(ALICE_DID)).toBe(false);
      expect(store.publishingVersion(ALICE_DID)).toBe(0);
      const columns = store.db.query<{ name: string; dflt_value: string | null }, []>("PRAGMA table_info(publication_consent)").all();
      expect(columns.find((c) => c.name === "enabled")?.dflt_value).toBe("0");
    } finally {
      store.close();
    }
  });

  test("legacy credentials are discarded without changing account links or identity sessions", () => {
    const dir = mkdtempSync(join(tmpdir(), "wrathplus-privacy-"));
    const path = join(dir, "account.sqlite");
    try {
      const legacy = new Database(path);
      legacy.exec(`
        CREATE TABLE account_link (did TEXT PRIMARY KEY, account TEXT NOT NULL UNIQUE,
          handle TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
        CREATE TABLE oauth_session (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at INTEGER NOT NULL);
      `);
      legacy.query("INSERT INTO account_link VALUES (?, 'ALICE', 'alice.test', 123, 456)").run(ALICE_DID);
      legacy.query("INSERT INTO oauth_session VALUES (?, '{}', 123)").run(ALICE_DID);
      legacy.close();
      const store = new Store(path);
      try {
        expect(store.publishingEnabled(ALICE_DID)).toBe(false);
        expect(store.publishingVersion(ALICE_DID)).toBe(0);
        expect(store.hasSession(ALICE_DID)).toBe(false);
        expect(store.linkByDid(ALICE_DID)).toEqual({ did: ALICE_DID, account: "ALICE", handle: "alice.test", createdAt: 123, updatedAt: 456 });
      } finally {
        store.close();
      }
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("disable synchronously deletes credentials but preserves link and web session", async () => {
    const store = new Store(":memory:");
    try {
      store.insertLink(ALICE_DID, "ALICE", "alice.test", "encrypted");
      const link = store.linkByDid(ALICE_DID);
      store.insertWebSession("hash", ALICE_DID, "alice.test", "csrf", Date.now() + 60_000);
      expect(store.setPublishing(ALICE_DID, true, 0)).toBe(true);
      await store.sessionStore().set(ALICE_DID, { refresh: "secret" });
      expect(store.hasSession(ALICE_DID)).toBe(true);
      expect(store.setPublishing(ALICE_DID, false)).toBe(true);
      expect(store.hasSession(ALICE_DID)).toBe(false);
      expect(await store.sessionStore().get(ALICE_DID)).toBeUndefined();
      expect(store.publishingEnabled(ALICE_DID)).toBe(false);
      expect(store.linkByDid(ALICE_DID)).toEqual(link);
      expect(store.webSession("hash")?.did).toBe(ALICE_DID);
    } finally {
      store.close();
    }
  });

  test("versions reject stale callbacks, including disable before first consent", () => {
    const store = new Store(":memory:");
    try {
      const before = store.publishingVersion(ALICE_DID);
      expect(store.setPublishing(ALICE_DID, false)).toBe(true);
      expect(store.publishingVersion(ALICE_DID)).toBe(1);
      expect(store.setPublishing(ALICE_DID, true, before)).toBe(false);
      expect(store.publishingEnabled(ALICE_DID)).toBe(false);
      expect(store.setPublishing(ALICE_DID, true, 1)).toBe(true);
      expect(store.publishingVersion(ALICE_DID)).toBe(2);
      expect(store.setPublishing(ALICE_DID, false)).toBe(true);
      expect(store.setPublishing(ALICE_DID, false)).toBe(true);
      expect(store.publishingVersion(ALICE_DID)).toBe(4);
      expect(store.setPublishing(ALICE_DID, true, 2)).toBe(false);
      expect(store.publishingVersion(ALICE_DID)).toBe(4);
    } finally {
      store.close();
    }
  });

  test("temporary identity callback storage is allowed; explicit forgetting does not change consent", async () => {
    const store = new Store(":memory:");
    try {
      await store.sessionStore().set(ALICE_DID, { temporary: true });
      expect(store.hasSession(ALICE_DID)).toBe(true);
      expect(store.publishingEnabled(ALICE_DID)).toBe(false);
      store.forgetOAuthSession(ALICE_DID);
      store.forgetOAuthSession(ALICE_DID);
      expect(store.hasSession(ALICE_DID)).toBe(false);
      expect(store.publishingVersion(ALICE_DID)).toBe(0);
      store.setPublishing(ALICE_DID, true);
      store.forgetOAuthSession(ALICE_DID);
      expect(store.publishingEnabled(ALICE_DID)).toBe(true);
    } finally {
      store.close();
    }
  });

  test("consent and version persist across restarts; only opted-in credentials survive", async () => {
    const dir = mkdtempSync(join(tmpdir(), "wrathplus-privacy-"));
    const path = join(dir, "account.sqlite");
    try {
      const store = new Store(path);
      store.setPublishing(ALICE_DID, true);
      await store.sessionStore().set(ALICE_DID, { refresh: "public" });
      await store.sessionStore().set("did:plc:private", { refresh: "temporary" });
      store.close();
      const reopened = new Store(path);
      try {
        expect(reopened.publishingEnabled(ALICE_DID)).toBe(true);
        expect(reopened.publishingVersion(ALICE_DID)).toBe(1);
        expect(await reopened.sessionStore().get(ALICE_DID)).toEqual({ refresh: "public" });
        expect(reopened.hasSession("did:plc:private")).toBe(false);
      } finally {
        reopened.close();
      }
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
