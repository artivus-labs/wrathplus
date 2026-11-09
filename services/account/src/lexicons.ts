// Publishing our lexicons (https://atproto.com/specs/lexicon#lexicon-publication-and-resolution):
// each schema becomes a `com.atproto.lexicon.schema` record, key = its NSID, in the repository of
// the account that will be named by the DNS TXT record `_lexicon.wrathplus.artivus.com.au`
// (`did=<that account's DID>`). This module does the record writes; the DNS record is printed for
// whoever owns the zone to create.

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { guardedJson, verifyHandle, type IdentityDeps } from "./identity.ts";

export const AUTHORITY = "wrathplus.artivus.com.au";
export const NSID_PREFIX = "au.com.artivus.wrathplus.";
const NSID = /^au\.com\.artivus\.wrathplus\.[a-zA-Z][a-zA-Z0-9]{0,62}$/;

export interface LexiconDoc {
  lexicon: 1;
  id: string;
  defs: Record<string, unknown>;
  [key: string]: unknown;
}

/** Every `<nsid>.json` in `dir`, checked: lexicon 1, id = file name, under our authority. */
export function loadLexicons(dir: string): LexiconDoc[] {
  return readdirSync(dir)
    .filter((f) => f.endsWith(".json"))
    .sort()
    .map((file) => {
      const doc = JSON.parse(readFileSync(join(dir, file), "utf8")) as Partial<LexiconDoc>;
      if (doc.lexicon !== 1) throw new Error(`${file}: "lexicon" must be 1`);
      if (typeof doc.id !== "string" || !NSID.test(doc.id) || `${doc.id}.json` !== file) {
        throw new Error(`${file}: "id" must be an NSID under ${NSID_PREFIX} matching the file name`);
      }
      if (typeof doc.defs !== "object" || doc.defs === null || !("main" in doc.defs)) throw new Error(`${file}: needs defs.main`);
      return doc as LexiconDoc;
    });
}

/** The DNS record that makes `did` the authority for our NSIDs. */
export function txtRecord(did: string): string {
  return `_lexicon.${AUTHORITY}. TXT "did=${did}"`;
}

/** Handle and app password from the environment or a 0600 file (`handle:app-password`). Never argv. */
export function readPublishCredentials(env: Record<string, string | undefined>): { handle: string; appPassword: string } {
  const file = env.LEXICON_PUBLISH_CREDENTIALS_FILE;
  if (file) {
    if (statSync(file).mode & 0o077) throw new Error(`${file} must not be readable by group or others (chmod 600)`);
    const line = readFileSync(file, "utf8").split("\n")[0]?.trim() ?? "";
    const at = line.indexOf(":");
    if (at <= 0 || at === line.length - 1) throw new Error(`${file} must hold handle:app-password on its first line`);
    return { handle: line.slice(0, at), appPassword: line.slice(at + 1) };
  }
  const handle = env.LEXICON_PUBLISH_HANDLE;
  const appPassword = env.LEXICON_PUBLISH_APP_PASSWORD;
  if (!handle || !appPassword) {
    throw new Error("set LEXICON_PUBLISH_CREDENTIALS_FILE, or LEXICON_PUBLISH_HANDLE and LEXICON_PUBLISH_APP_PASSWORD");
  }
  return { handle, appPassword };
}

export interface PublishResult {
  did: string;
  pds: string;
  published: Array<{ nsid: string; uri: string }>;
}

/** Sign in with the app password, put every schema, sign out. Throws on the first failure. */
export async function publishLexicons(
  credentials: { handle: string; appPassword: string },
  lexicons: LexiconDoc[],
  deps: IdentityDeps,
): Promise<PublishResult> {
  const identity = await verifyHandle(credentials.handle, deps);
  const session = await guardedJson(`${identity.pds}/xrpc/com.atproto.server.createSession`, deps, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ identifier: identity.handle, password: credentials.appPassword }),
  });
  const s = session.json as { did?: unknown; accessJwt?: unknown; refreshJwt?: unknown } | undefined;
  if (session.status !== 200 || typeof s?.accessJwt !== "string") throw new Error(`createSession failed (HTTP ${session.status})`);
  try {
    if (s.did !== identity.did) throw new Error("the PDS signed in a different account than the handle names");
    const published: PublishResult["published"] = [];
    for (const doc of lexicons) {
      const res = await guardedJson(`${identity.pds}/xrpc/com.atproto.repo.putRecord`, deps, {
        method: "POST",
        headers: { "content-type": "application/json", authorization: `Bearer ${s.accessJwt}` },
        body: JSON.stringify({
          repo: identity.did,
          collection: "com.atproto.lexicon.schema",
          rkey: doc.id,
          record: { $type: "com.atproto.lexicon.schema", ...doc },
        }),
      });
      const out = res.json as { uri?: unknown; message?: unknown } | undefined;
      if (res.status !== 200) throw new Error(`putRecord ${doc.id} failed (HTTP ${res.status}): ${String(out?.message ?? "")}`);
      published.push({ nsid: doc.id, uri: typeof out?.uri === "string" ? out.uri : `at://${identity.did}/com.atproto.lexicon.schema/${doc.id}` });
    }
    return { did: identity.did, pds: identity.pds, published };
  } finally {
    if (typeof s.refreshJwt === "string") {
      await guardedJson(`${identity.pds}/xrpc/com.atproto.server.deleteSession`, deps, {
        method: "POST",
        headers: { authorization: `Bearer ${s.refreshJwt}` },
      }).catch(() => undefined);
    }
  }
}
