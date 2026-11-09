import { describe, expect, test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadLexicons, publishLexicons, readPublishCredentials, txtRecord } from "../src/lexicons.ts";
import { addIdentity, depsFor, fakeNet } from "./fakes.ts";

const DIR = join(import.meta.dir, "..", "lexicons");
const SKEIN = "did:plc:skeinskeinskeinskeinskei";
const PDS = "https://pds.delve.town";

describe("loadLexicons", () => {
  test("our two schemas load and are well-formed", () => {
    const docs = loadLexicons(DIR);
    expect(docs.map((d) => d.id)).toEqual(["au.com.artivus.wrathplus.character", "au.com.artivus.wrathplus.milestone"]);
    for (const d of docs) {
      const main = d.defs.main as { type: string; key: string; record: { required: string[]; properties: Record<string, unknown> } };
      expect(main.type).toBe("record");
      for (const field of main.record.required) expect(main.record.properties[field]).toBeDefined();
    }
  });
  test("a file whose id does not match its name, or outside our authority, is refused", () => {
    const dir = mkdtempSync(join(tmpdir(), "wp-lex-"));
    writeFileSync(join(dir, "au.com.artivus.wrathplus.x.json"), JSON.stringify({ lexicon: 1, id: "au.com.artivus.wrathplus.y", defs: { main: {} } }));
    expect(() => loadLexicons(dir)).toThrow(/file name/);
    const other = mkdtempSync(join(tmpdir(), "wp-lex-"));
    writeFileSync(join(other, "com.example.x.json"), JSON.stringify({ lexicon: 1, id: "com.example.x", defs: { main: {} } }));
    expect(() => loadLexicons(other)).toThrow(/under/);
  });
});

describe("credentials", () => {
  test("from the environment or a 0600 file", () => {
    expect(readPublishCredentials({ LEXICON_PUBLISH_HANDLE: "skein.example.com", LEXICON_PUBLISH_APP_PASSWORD: "a-b-c-d" })).toEqual({
      handle: "skein.example.com",
      appPassword: "a-b-c-d",
    });
    const dir = mkdtempSync(join(tmpdir(), "wp-cred-"));
    const file = join(dir, "c");
    writeFileSync(file, "skein.example.com:a-b-c-d\n", { mode: 0o600 });
    expect(readPublishCredentials({ LEXICON_PUBLISH_CREDENTIALS_FILE: file }).appPassword).toBe("a-b-c-d");
    const open = join(dir, "open");
    writeFileSync(open, "x:y\n", { mode: 0o644 });
    expect(() => readPublishCredentials({ LEXICON_PUBLISH_CREDENTIALS_FILE: open })).toThrow(/chmod 600/);
    expect(() => readPublishCredentials({})).toThrow();
  });
});

describe("publishLexicons", () => {
  test("puts each schema at com.atproto.lexicon.schema/<nsid> and signs out", async () => {
    const net = fakeNet();
    addIdentity(net, { did: SKEIN, handle: "skein.example.com", pds: PDS });
    const puts: Array<{ auth: string | null; body: Record<string, unknown> }> = [];
    net.routes[`POST ${PDS}/xrpc/com.atproto.repo.putRecord`] = async (req) => {
      const body = (await req.json()) as Record<string, unknown>;
      puts.push({ auth: req.headers.get("authorization"), body });
      return Response.json({ uri: `at://${SKEIN}/com.atproto.lexicon.schema/${body.rkey}`, cid: "bafy" });
    };
    const docs = loadLexicons(DIR);
    const result = await publishLexicons({ handle: "skein.example.com", appPassword: "aaaa-bbbb-cccc-dddd" }, docs, depsFor(net));
    expect(result.did).toBe(SKEIN);
    expect(puts.map((p) => p.body.rkey)).toEqual(docs.map((d) => d.id));
    for (const p of puts) {
      expect(p.auth).toStartWith("Bearer ");
      expect(p.body).toMatchObject({ repo: SKEIN, collection: "com.atproto.lexicon.schema" });
      const record = p.body.record as Record<string, unknown>;
      expect(record.$type).toBe("com.atproto.lexicon.schema");
      expect(record.id).toBe(p.body.rkey);
      expect(record.lexicon).toBe(1);
    }
    expect(net.requests.some((r) => new URL(r.url).pathname.endsWith("deleteSession"))).toBe(true);
    expect(txtRecord(result.did)).toBe(`_lexicon.wrathplus.artivus.com.au. TXT "did=${SKEIN}"`);
  });

  test("a rejected app password publishes nothing", async () => {
    const net = fakeNet();
    addIdentity(net, { did: SKEIN, handle: "skein.example.com", pds: PDS });
    await expect(publishLexicons({ handle: "skein.example.com", appPassword: "wrong" }, loadLexicons(DIR), depsFor(net))).rejects.toThrow(/createSession/);
    expect(net.requests.some((r) => new URL(r.url).pathname.endsWith("putRecord"))).toBe(false);
  });
});
