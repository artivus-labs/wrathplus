import { describe, expect, test } from "bun:test";
import { accountNameCandidates, ACCOUNT_NAME, cleanHandle, firstFreeCandidate } from "../src/naming.ts";
import { ALICE_DID, BOB_DID } from "./fakes.ts";

describe("cleanHandle", () => {
  test("drops the shared hosting suffix, dots and hyphens, upper-cases", () => {
    expect(cleanHandle("alice.bsky.social")).toBe("ALICE");
    expect(cleanHandle("@Bob-Smith.Example.com")).toBe("BOBSMITHEXAMPLECOM");
    expect(cleanHandle("pds.delve.town", ["delve.town"])).toBe("PDS");
  });
  test("a handle that is only the suffix keeps it", () => {
    expect(cleanHandle("bsky.social")).toBe("BSKYSOCIAL");
  });
});

describe("accountNameCandidates", () => {
  test("first choice is the cleaned handle; every candidate is a valid AC name", () => {
    const c = accountNameCandidates("alice.bsky.social", ALICE_DID);
    expect(c[0]).toBe("ALICE");
    expect(c.length).toBe(9);
    for (const name of c) expect(name).toMatch(ACCOUNT_NAME);
  });

  test("long handles are cut to 16; hashed fallbacks keep 11 + 5", () => {
    const c = accountNameCandidates("averyveryverylonghandle.example.com", ALICE_DID);
    expect(c[0]).toBe("AVERYVERYVERYLON");
    expect(c[0]!.length).toBe(16);
    for (const name of c.slice(1)) {
      expect(name.length).toBe(16);
      expect(name.startsWith("AVERYVERYVE")).toBe(true);
    }
  });

  test("deterministic in (handle, DID)", () => {
    expect(accountNameCandidates("alice.bsky.social", ALICE_DID)).toEqual(accountNameCandidates("ALICE.bsky.social", ALICE_DID));
  });

  test("two DIDs with colliding handles share only the first choice", () => {
    const a = accountNameCandidates("alice.bsky.social", ALICE_DID);
    const b = accountNameCandidates("alice.bsky.social", BOB_DID);
    expect(a[0]).toBe(b[0]!);
    expect(a.slice(1).filter((n) => b.includes(n))).toEqual([]);
  });

  test("truncation collisions resolve deterministically to distinct names", () => {
    const a = accountNameCandidates("sixteencharsxxxxA.example.com", ALICE_DID);
    const b = accountNameCandidates("sixteencharsxxxxB.example.com", BOB_DID);
    expect(a[0]).toBe("SIXTEENCHARSXXXX");
    expect(b[0]).toBe("SIXTEENCHARSXXXX");
    const taken = new Set([a[0]!]);
    const bName = firstFreeCandidate(b, (n) => taken.has(n));
    expect(bName).not.toBe(a[0]!);
    expect(bName).toBe(firstFreeCandidate(b, (n) => taken.has(n))!);
  });

  test("reserved prefixes (mod-playerbots counts RNDBOT%) are never produced", () => {
    const c = accountNameCandidates("rndbot42.bsky.social", ALICE_DID);
    for (const name of c) expect(name.startsWith("RNDBOT")).toBe(false);
    expect(c[0]!.startsWith("ATRNDBOT42")).toBe(true);
    const custom = accountNameCandidates("guest.example.com", ALICE_DID, { reservedPrefixes: ["GUEST"] });
    for (const name of custom) expect(name.startsWith("GUEST")).toBe(false);
  });

  test("never contains anything but A-Z0-9, whatever the handle", () => {
    for (const handle of ["a.b", "x--y.example.com", "0day.example.com", "日本.example.com"]) {
      for (const name of accountNameCandidates(handle, ALICE_DID)) expect(name).toMatch(ACCOUNT_NAME);
    }
  });
});
