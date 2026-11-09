import { describe, expect, test } from "bun:test";
import { chmodSync, mkdtempSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadOrCreateServiceKeys, PasswordCipher } from "../src/secrets.ts";
import { ALICE_DID, BOB_DID, TEST_KEY } from "./fakes.ts";

describe("service keys", () => {
  test("generated once into a 0600 file, reloaded identically, refused when readable", () => {
    const path = join(mkdtempSync(join(tmpdir(), "wp-keys-")), "service-keys.json");
    const a = loadOrCreateServiceKeys(path);
    expect(statSync(path).mode & 0o777).toBe(0o600);
    expect(a.password.length).toBe(32);
    expect(a.cookie.length).toBe(32);
    expect(a.password.equals(a.cookie)).toBe(false);
    const b = loadOrCreateServiceKeys(path);
    expect(b.password.equals(a.password)).toBe(true);
    chmodSync(path, 0o644);
    expect(() => loadOrCreateServiceKeys(path)).toThrow(/chmod 600/);
  });
});

describe("PasswordCipher (AES-256-GCM)", () => {
  const cipher = new PasswordCipher(TEST_KEY);
  test("round-trips, with a fresh IV each time", () => {
    const x = cipher.encrypt(ALICE_DID, "ABCDEFGHJKLMNPQR");
    const y = cipher.encrypt(ALICE_DID, "ABCDEFGHJKLMNPQR");
    expect(x).not.toBe(y);
    expect(cipher.decrypt(ALICE_DID, x)).toBe("ABCDEFGHJKLMNPQR");
  });
  test("bound to the DID, the key and the bytes", () => {
    const x = cipher.encrypt(ALICE_DID, "ABCDEFGHJKLMNPQR");
    expect(cipher.decrypt(BOB_DID, x)).toBeUndefined();
    expect(new PasswordCipher(Buffer.alloc(32, 1)).decrypt(ALICE_DID, x)).toBeUndefined();
    const raw = Buffer.from(x.slice(3), "base64url");
    raw[14] = raw[14]! ^ 1;
    expect(cipher.decrypt(ALICE_DID, `v1:${raw.toString("base64url")}`)).toBeUndefined();
    expect(cipher.decrypt(ALICE_DID, "v0:abc")).toBeUndefined();
    expect(cipher.decrypt(ALICE_DID, "v1:")).toBeUndefined();
  });
});
