import { describe, expect, test } from "bun:test";
import { chmodSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { commands, envelope, parseSoapResponse, readSoapCredentials, soapClient, SoapError } from "../src/soap.ts";
import { ALICE_DID } from "./fakes.ts";

const PASSWORD = "ABCDEFGHJKLMNPQR";

describe("command building", () => {
  test("well-formed commands", () => {
    expect(commands.createAccount("ALICE", PASSWORD)).toBe(`.account create ALICE ${PASSWORD}`);
    expect(commands.setPassword("ALICE", PASSWORD)).toBe(`.account set password ALICE ${PASSWORD} ${PASSWORD}`);
    expect(commands.setAddon("ALICE")).toBe(".account set addon ALICE 2");
    expect(commands.link("ALICE", ALICE_DID, "alice.bsky.social")).toBe(`.atproto link ALICE ${ALICE_DID} alice.bsky.social`);
    expect(commands.events(50)).toBe(".atproto events 50");
    expect(commands.ack(12)).toBe(".atproto ack 12");
  });

  test("handles cannot inject arguments or commands", () => {
    const evil = [
      "alice.bsky.social 2",
      "alice.bsky.social\n.account set gmlevel ALICE 3",
      "alice.bsky.social;.server shutdown 1",
      "Alice.bsky.social",
      "<x>.example.com",
      "a'b.example.com",
      "alice.local",
      "",
    ];
    for (const handle of evil) expect(() => commands.link("ALICE", ALICE_DID, handle)).toThrow();
  });

  test("names, passwords, DIDs and numbers are whitelisted", () => {
    expect(() => commands.createAccount("alice", PASSWORD)).toThrow();
    expect(() => commands.createAccount("ALICE X", PASSWORD)).toThrow();
    expect(() => commands.createAccount("SEVENTEENCHARSXXX", PASSWORD)).toThrow();
    expect(() => commands.createAccount("ALICE", "short")).toThrow();
    expect(() => commands.createAccount("ALICE", `${PASSWORD.slice(0, 15)} `)).toThrow();
    expect(() => commands.link("ALICE", "did:plc:short", "alice.bsky.social")).toThrow();
    expect(() => commands.link("ALICE", "did:web:evil.com 1", "alice.bsky.social")).toThrow();
    expect(() => commands.events(-1)).toThrow();
    expect(() => commands.events(1.5)).toThrow();
    expect(() => commands.ack(Number.NaN)).toThrow();
  });

  test("the envelope escapes XML and refuses multi-line commands", () => {
    const xml = envelope(".x <a> & 'b' \"c\"");
    expect(xml).toContain("<command>.x &lt;a&gt; &amp; &apos;b&apos; &quot;c&quot;</command>");
    expect(() => envelope(".a\n.b")).toThrow();
  });
});

describe("responses", () => {
  test("result text is unescaped", () => {
    const body = "<SOAP-ENV:Body><ns1:executeCommandResponse><result>EV 1 &amp; 2&#xA;line</result></ns1:executeCommandResponse>";
    expect(parseSoapResponse(200, body)).toBe("EV 1 & 2\nline");
  });
  test("faults become SoapErrors with the realm's message", () => {
    const body = "<faultstring>Account already exist!&#xA;</faultstring>";
    const err = (() => {
      try {
        parseSoapResponse(500, body);
      } catch (e) {
        return e as SoapError;
      }
    })();
    expect(err).toBeInstanceOf(SoapError);
    expect(err!.alreadyExists).toBe(true);
    expect(() => parseSoapResponse(401, "")).toThrow(/refused/);
  });

  test("client posts the envelope with basic auth", async () => {
    let seen: Request | undefined;
    const run = soapClient(
      "http://127.0.0.1:17878/",
      () => ({ user: "SOAPUSER", password: "pw" }),
      (async (input: string | URL | Request, init?: RequestInit) => {
        seen = (input instanceof Request ? new Request(input, init) : new Request(input.toString(), init));
        return new Response("<result>ok</result>", { status: 200 });
      }) as typeof fetch,
    );
    expect(await run(".atproto events 5")).toBe("ok");
    expect(seen!.headers.get("authorization")).toBe(`Basic ${Buffer.from("SOAPUSER:pw").toString("base64")}`);
    expect(await seen!.text()).toContain("<command>.atproto events 5</command>");
  });
});

describe("credentials file", () => {
  const dir = mkdtempSync(join(tmpdir(), "wp-soap-"));
  test("reads USER:PASSWORD from a 0600 file", () => {
    const p = join(dir, "ok");
    writeFileSync(p, "SOAPUSER:se:cret\n", { mode: 0o600 });
    expect(readSoapCredentials(p)).toEqual({ user: "SOAPUSER", password: "se:cret" });
  });
  test("refuses a group- or world-readable file", () => {
    const p = join(dir, "open");
    writeFileSync(p, "SOAPUSER:pw\n");
    chmodSync(p, 0o644);
    expect(() => readSoapCredentials(p)).toThrow(/chmod 600/);
  });
});
