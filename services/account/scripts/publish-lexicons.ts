// Publish lexicons/*.json as com.atproto.lexicon.schema records (rkey = NSID) in the repository of
// the account given by credentials, then print the DNS TXT record that points our NSID authority
// at that account. It never creates the DNS record.
//
// Credentials come from the environment, never argv:
//   LEXICON_PUBLISH_CREDENTIALS_FILE=/path/to/file   (0600; first line "handle:app-password")
// or
//   LEXICON_PUBLISH_HANDLE=<handle> LEXICON_PUBLISH_APP_PASSWORD=<app password>
//
//   bun run scripts/publish-lexicons.ts            # publish
//   bun run scripts/publish-lexicons.ts --dry-run  # validate and list; no network

import { join } from "node:path";
import { nodeIdentityDeps } from "../src/identity.ts";
import { loadLexicons, publishLexicons, readPublishCredentials, txtRecord } from "../src/lexicons.ts";

const args = process.argv.slice(2);
if (args.some((a) => a !== "--dry-run")) {
  console.error("usage: publish-lexicons.ts [--dry-run]   (credentials only via the environment; see the file header)");
  process.exit(2);
}

const lexicons = loadLexicons(join(import.meta.dir, "..", "lexicons"));
console.log(`lexicons: ${lexicons.map((l) => l.id).join(", ")}`);
if (args.includes("--dry-run")) {
  console.log("dry run: nothing published");
  process.exit(0);
}

const credentials = readPublishCredentials(process.env);
const deps = await nodeIdentityDeps(process.env.PLC_URL || "https://plc.directory");
const result = await publishLexicons(credentials, lexicons, deps);
console.log(`published as ${credentials.handle} (${result.did}) on ${result.pds}:`);
for (const p of result.published) console.log(`  ${p.nsid} -> ${p.uri}`);
console.log("\nCreate this DNS record (not created by this script):");
console.log(`  ${txtRecord(result.did)}`);
