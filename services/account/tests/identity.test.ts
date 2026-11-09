import { describe, expect, test } from "bun:test";
import { assertPublicUrl, isDid, isPrivateAddress, normalizeHandle, resolveHandle, verifiedHandleForDid, verifyHandle } from "../src/identity.ts";
import { addIdentity, ALICE_DID, BOB_DID, depsFor, didDoc, fakeNet } from "./fakes.ts";

describe("syntax", () => {
  test("handles", () => {
    expect(normalizeHandle("@Alice.BSKY.social")).toBe("alice.bsky.social");
    for (const bad of ["alice", "alice..com", "-a.com", "a.123", "a.local", "a b.com", "a_b.com", "x".repeat(254) + ".com"]) {
      expect(normalizeHandle(bad)).toBeUndefined();
    }
  });
  test("DIDs", () => {
    expect(isDid(ALICE_DID)).toBe(true);
    expect(isDid("did:web:example.com")).toBe(true);
    expect(isDid("did:web:localhost")).toBe(false);
    expect(isDid("did:plc:ABCDEFGHIJKLMNOPQRSTUVWX")).toBe(false);
    expect(isDid("did:key:z6Mk")).toBe(false);
  });
  test("private addresses", () => {
    for (const ip of ["127.0.0.1", "10.1.2.3", "172.20.0.1", "192.168.1.1", "169.254.169.254", "100.64.0.1", "0.0.0.0", "::1", "fd00::1", "fe80::1", "::ffff:127.0.0.1"]) {
      expect(isPrivateAddress(ip)).toBe(true);
    }
    for (const ip of ["2.31.195.57", "8.8.8.8", "2a01:4f8:c015:dc2c::1"]) expect(isPrivateAddress(ip)).toBe(false);
  });
});

describe("public URL guard", () => {
  const lookup = async (host: string) => (({ "good.example.com": ["93.184.216.34"], "rebind.example.com": ["127.0.0.1"] }) as Record<string, string[]>)[host] ?? [];
  test("https on a public host passes", async () => {
    expect((await assertPublicUrl("https://good.example.com/x", { lookup })).host).toBe("good.example.com");
  });
  test("http, loopback, private IPs and private DNS answers are refused", async () => {
    for (const url of ["http://good.example.com", "https://127.0.0.1:17878/", "https://[::1]/", "https://localhost/", "https://rebind.example.com/", "https://u:p@good.example.com/"]) {
      await expect(assertPublicUrl(url, { lookup })).rejects.toThrow();
    }
  });
});

describe("resolution", () => {
  test("DNS TXT handle -> DID -> document claiming the handle", async () => {
    const net = fakeNet();
    addIdentity(net, { did: ALICE_DID, handle: "alice.bsky.social" });
    const id = await verifyHandle("Alice.bsky.social", depsFor(net));
    expect(id).toEqual({ did: ALICE_DID, handle: "alice.bsky.social", pds: "https://pds.example.com" });
  });

  test("well-known fallback when there is no TXT record", async () => {
    const net = fakeNet();
    net.hosts["alice.example.com"] = ["93.184.216.34"];
    net.routes["GET https://alice.example.com/.well-known/atproto-did"] = () => new Response(`${ALICE_DID}\n`);
    expect(await resolveHandle("alice.example.com", depsFor(net))).toBe(ALICE_DID);
  });

  test("conflicting TXT records fail", async () => {
    const net = fakeNet();
    net.txt["_atproto.alice.example.com"] = [`did=${ALICE_DID}`, `did=${BOB_DID}`];
    await expect(resolveHandle("alice.example.com", depsFor(net))).rejects.toThrow(/conflicting/);
  });

  test("a document that does not claim the handle is a mismatch (handle squatting)", async () => {
    const net = fakeNet();
    addIdentity(net, { did: ALICE_DID, handle: "alice.bsky.social" });
    net.txt["_atproto.mallory.example.com"] = [`did=${ALICE_DID}`];
    await expect(verifyHandle("mallory.example.com", depsFor(net))).rejects.toThrow(/does not claim/);
  });

  test("a PDS on a private address is refused", async () => {
    const net = fakeNet();
    addIdentity(net, { did: ALICE_DID, handle: "alice.bsky.social", pds: "https://pds.evil.com" });
    net.hosts["pds.evil.com"] = ["127.0.0.1"];
    await expect(verifyHandle("alice.bsky.social", depsFor(net))).rejects.toThrow(/private/);
  });

  test("a PDS over http is refused", async () => {
    const net = fakeNet();
    net.txt["_atproto.alice.bsky.social"] = [`did=${ALICE_DID}`];
    net.hosts["plc.directory"] = ["104.16.0.1"];
    net.routes[`GET https://plc.directory/${ALICE_DID}`] = () => Response.json(didDoc(ALICE_DID, "alice.bsky.social", "http://127.0.0.1:17878"));
    await expect(verifyHandle("alice.bsky.social", depsFor(net))).rejects.toThrow(/https/);
  });

  test("DID -> verified handle (bidirectional)", async () => {
    const net = fakeNet();
    addIdentity(net, { did: ALICE_DID, handle: "alice.bsky.social" });
    expect(await verifiedHandleForDid(ALICE_DID, depsFor(net))).toBe("alice.bsky.social");
    // The document claims a handle whose DNS points elsewhere: no verified handle.
    net.txt["_atproto.alice.bsky.social"] = [`did=${BOB_DID}`];
    expect(await verifiedHandleForDid(ALICE_DID, depsFor(net))).toBeUndefined();
  });
});
