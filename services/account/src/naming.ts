// Game account names for ATProto identities.
//
// AzerothCore account names are at most 16 characters and the realm upper-cases them; we only
// ever use A-Z and 0-9, so a name is also safe as a SOAP command token. The DID is the permanent
// key: a name is derived once, stored against the DID, and never changes after that, even if the
// handle does.
//
// The candidates for one identity are a fixed list that depends only on (handle, DID):
//   0. the cleaned handle, cut to 16 ("alice.bsky.social" -> "ALICE", "bob.example.com" ->
//      "BOBEXAMPLECOM");
//   1..N. the cleaned handle cut to 11, plus 5 characters of base32(sha256(DID + "#" + k)).
// The first candidate that is neither stored for another DID nor already an account on the realm
// wins. Two people whose handles clean to the same name therefore get different, stable names, and
// an account we did not create (a bot, an admin, a realm owner's own) is never taken over.
// Names under a reserved prefix (RNDBOT: mod-playerbots counts `username LIKE 'RNDBOT%'` as its
// own) get "AT" in front instead.

import { createHash } from "node:crypto";

export const ACCOUNT_NAME_MAX = 16;
export const ACCOUNT_NAME = /^[A-Z0-9]{1,16}$/;
const SUFFIX_LENGTH = 5;
const HASHED_CANDIDATES = 8;

export const DEFAULT_RESERVED_PREFIXES = ["RNDBOT", "AHBOT", "WPMANAGER", "ADMIN", "SOAP"];
export const DEFAULT_HOSTED_SUFFIXES = ["bsky.social"];

export interface NamingOptions {
  /** Upper-case prefixes no derived name may start with. */
  reservedPrefixes?: readonly string[];
  /** Shared hosting domains dropped from the end of a handle before cleaning. */
  hostedSuffixes?: readonly string[];
}

const BASE32 = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

function base32(bytes: Uint8Array): string {
  let out = "";
  let bits = 0;
  let value = 0;
  for (const byte of bytes) {
    value = (value << 8) | byte;
    bits += 8;
    while (bits >= 5) {
      out += BASE32[(value >>> (bits - 5)) & 31];
      bits -= 5;
    }
    value &= (1 << bits) - 1;
  }
  if (bits > 0) out += BASE32[(value << (5 - bits)) & 31];
  return out;
}

/** The handle reduced to upper-case letters and digits, without a shared hosting suffix. */
export function cleanHandle(handle: string, hostedSuffixes: readonly string[] = DEFAULT_HOSTED_SUFFIXES): string {
  let h = handle.trim().toLowerCase().replace(/^@/, "");
  for (const suffix of hostedSuffixes) {
    const s = `.${suffix.toLowerCase()}`;
    if (h.endsWith(s) && h.length > s.length) {
      h = h.slice(0, -s.length);
      break;
    }
  }
  return h.replace(/[^a-z0-9]/g, "").toUpperCase();
}

function isReserved(name: string, reserved: readonly string[]): boolean {
  return reserved.some((p) => p && name.startsWith(p.toUpperCase()));
}

/** Every name this identity may get, in order of preference. Deterministic in (handle, DID). */
export function accountNameCandidates(handle: string, did: string, options: NamingOptions = {}): string[] {
  const reserved = options.reservedPrefixes ?? DEFAULT_RESERVED_PREFIXES;
  let base = cleanHandle(handle, options.hostedSuffixes ?? DEFAULT_HOSTED_SUFFIXES);
  const out: string[] = [];
  if (base && !isReserved(base, reserved)) out.push(base.slice(0, ACCOUNT_NAME_MAX));
  if (isReserved(base, reserved)) base = `AT${base}`;
  const stem = base.slice(0, ACCOUNT_NAME_MAX - SUFFIX_LENGTH);
  for (let k = 1; k <= HASHED_CANDIDATES; k++) {
    const digest = createHash("sha256").update(`${did}#${k}`).digest();
    const name = `${stem}${base32(digest).slice(0, SUFFIX_LENGTH)}`;
    // An empty stem gives a 5-character hash-only name; prefix it so it reads as ours.
    const candidate = stem ? name : `AT${name}`;
    if (!isReserved(candidate, reserved) && !out.includes(candidate)) out.push(candidate);
  }
  for (const name of out) {
    if (!ACCOUNT_NAME.test(name)) throw new Error(`derived an invalid account name: ${name}`);
  }
  return out;
}

/**
 * Pick the first candidate that is free. `isTaken` answers for names we already stored; creating
 * the account on the realm (which may say it already exists) is the caller's job, and the caller
 * moves on to the next candidate when it does.
 */
export function firstFreeCandidate(candidates: readonly string[], isTaken: (name: string) => boolean): string | undefined {
  return candidates.find((name) => !isTaken(name));
}
