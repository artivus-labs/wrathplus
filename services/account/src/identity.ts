// ATProto identity: handle and DID syntax, handle -> DID -> DID document -> PDS, checked in both
// directions (the document must list the handle back in alsoKnownAs). Everything that touches
// the network is injected, so tests run against stubs.
//
// Every URL we fetch on someone else's say-so (a handle's well-known file, a did:web document, a
// PDS) must be https on a public host: the service sits next to the realm's SOAP port and other
// loopback services, and a crafted DID document must not point our requests at them.

import { isIP } from "node:net";

export type Did = `did:plc:${string}` | `did:web:${string}`;

const DID_PLC = /^did:plc:[a-z2-7]{24}$/;
const DID_WEB = /^did:web:[a-z0-9.-]{1,253}$/;
const LABEL = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/;
const FORBIDDEN_TLDS = new Set(["alt", "arpa", "example", "internal", "invalid", "local", "localhost", "onion"]);
const MAX_BODY = 64 * 1024;

export class IdentityError extends Error {
  constructor(
    message: string,
    readonly code: "invalid_handle" | "invalid_did" | "unresolved" | "mismatch" | "unsafe_url",
  ) {
    super(message);
  }
}

/** A handle in canonical form (lower case, no leading @), or undefined if it is not a valid handle. */
export function normalizeHandle(input: string): string | undefined {
  const h = input.trim().replace(/^@/, "").toLowerCase();
  if (h.length < 3 || h.length > 253) return undefined;
  const labels = h.split(".");
  if (labels.length < 2) return undefined;
  if (!labels.every((l) => LABEL.test(l))) return undefined;
  const tld = labels[labels.length - 1]!;
  if (/^[0-9]/.test(tld) || FORBIDDEN_TLDS.has(tld)) return undefined;
  return h;
}

export function isDid(value: string): value is Did {
  return DID_PLC.test(value) || (DID_WEB.test(value) && normalizeHandle(value.slice("did:web:".length)) !== undefined);
}

// --- public-host guard -------------------------------------------------------------------------

function ipv4Private(ip: string): boolean {
  const p = ip.split(".").map(Number);
  const [a = 0, b = 0, c = 0] = p;
  return (
    a === 0 ||
    a === 10 ||
    a === 127 ||
    a >= 224 ||
    (a === 100 && b >= 64 && b <= 127) ||
    (a === 169 && b === 254) ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168) ||
    (a === 192 && b === 0 && c === 0) ||
    (a === 198 && (b === 18 || b === 19))
  );
}

/** Loopback, private, link-local, CGNAT, multicast and the like: anything not on the internet. */
export function isPrivateAddress(ip: string): boolean {
  const kind = isIP(ip);
  if (kind === 4) return ipv4Private(ip);
  if (kind !== 6) return true;
  const v6 = ip.toLowerCase();
  const mapped = /^(?:0*:)*:?ffff:(\d+\.\d+\.\d+\.\d+)$/.exec(v6) ?? /^::(\d+\.\d+\.\d+\.\d+)$/.exec(v6);
  if (mapped) return ipv4Private(mapped[1]!);
  if (v6 === "::" || v6 === "::1") return true;
  const first = parseInt(v6.split(":")[0] || "0", 16);
  return (first & 0xfe00) === 0xfc00 || (first & 0xffc0) === 0xfe80 || (first & 0xff00) === 0xff00;
}

export interface IdentityDeps {
  fetch: typeof fetch;
  /** DNS TXT lookup (node:dns/promises resolveTxt). */
  resolveTxt: (name: string) => Promise<string[][]>;
  /** All addresses of a host name (node:dns/promises lookup with all: true). */
  lookup: (host: string) => Promise<string[]>;
  plcUrl: string;
  timeoutMs?: number;
}

/** Throws unless `url` is https on a host that resolves only to public addresses. */
export async function assertPublicUrl(url: string, deps: Pick<IdentityDeps, "lookup">): Promise<URL> {
  let u: URL;
  try {
    u = new URL(url);
  } catch {
    throw new IdentityError(`not a URL: ${url}`, "unsafe_url");
  }
  if (u.protocol !== "https:") throw new IdentityError(`only https is allowed: ${u.origin}`, "unsafe_url");
  if (u.username || u.password) throw new IdentityError("credentials in URL", "unsafe_url");
  const host = u.hostname.replace(/^\[|\]$/g, "");
  if (isIP(host)) {
    if (isPrivateAddress(host)) throw new IdentityError(`private address: ${host}`, "unsafe_url");
    return u;
  }
  if (normalizeHandle(host) === undefined) throw new IdentityError(`not a public host name: ${host}`, "unsafe_url");
  const addresses = await deps.lookup(host).catch(() => [] as string[]);
  if (addresses.length === 0) throw new IdentityError(`host does not resolve: ${host}`, "unsafe_url");
  if (addresses.some(isPrivateAddress)) throw new IdentityError(`host resolves to a private address: ${host}`, "unsafe_url");
  return u;
}

async function guardedText(url: string, deps: IdentityDeps, init: RequestInit = {}): Promise<{ status: number; text: string }> {
  const u = await assertPublicUrl(url, deps);
  const res = await deps.fetch(u, {
    ...init,
    redirect: "manual",
    signal: AbortSignal.timeout(deps.timeoutMs ?? 8000),
  });
  const body = await res.arrayBuffer();
  if (body.byteLength > MAX_BODY) throw new IdentityError(`response too large from ${u.host}`, "unresolved");
  return { status: res.status, text: new TextDecoder().decode(body) };
}

export async function guardedJson(url: string, deps: IdentityDeps, init: RequestInit = {}): Promise<{ status: number; json: unknown }> {
  const { status, text } = await guardedText(url, deps, init);
  try {
    return { status, json: JSON.parse(text) };
  } catch {
    return { status, json: undefined };
  }
}

// --- resolution --------------------------------------------------------------------------------

/** Handle -> DID: DNS `_atproto.<handle>` TXT first, then https://<handle>/.well-known/atproto-did. */
export async function resolveHandle(handle: string, deps: IdentityDeps): Promise<Did> {
  const h = normalizeHandle(handle);
  if (!h) throw new IdentityError(`not a valid handle: ${handle}`, "invalid_handle");
  const records = await deps.resolveTxt(`_atproto.${h}`).catch(() => [] as string[][]);
  const fromDns = [...new Set(records.map((chunks) => chunks.join("")).filter((t) => t.startsWith("did=")).map((t) => t.slice(4).trim()))];
  if (fromDns.length > 1) throw new IdentityError(`handle has conflicting DNS records: ${h}`, "unresolved");
  if (fromDns.length === 1) {
    const did = fromDns[0]!;
    if (!isDid(did)) throw new IdentityError(`handle's DNS record is not a supported DID: ${h}`, "unresolved");
    return did;
  }
  const res = await guardedText(`https://${h}/.well-known/atproto-did`, deps).catch((e) => {
    if (e instanceof IdentityError && e.code === "unsafe_url") throw e;
    throw new IdentityError(`could not resolve handle ${h}`, "unresolved");
  });
  const did = res.text.trim();
  if (res.status !== 200 || !isDid(did)) throw new IdentityError(`could not resolve handle ${h}`, "unresolved");
  return did;
}

export interface DidDocument {
  id: string;
  alsoKnownAs: string[];
  pds: string;
}

/** DID -> document (PLC directory for did:plc, https://<host>/.well-known/did.json for did:web). */
export async function resolveDid(did: string, deps: IdentityDeps): Promise<DidDocument> {
  if (!isDid(did)) throw new IdentityError(`not a supported DID: ${did}`, "invalid_did");
  const url = did.startsWith("did:plc:")
    ? `${deps.plcUrl.replace(/\/$/, "")}/${did}`
    : `https://${did.slice("did:web:".length)}/.well-known/did.json`;
  const { status, json } = await guardedJson(url, deps).catch((e) => {
    if (e instanceof IdentityError && e.code === "unsafe_url") throw e;
    throw new IdentityError(`could not resolve ${did}`, "unresolved");
  });
  if (status !== 200 || typeof json !== "object" || json === null) throw new IdentityError(`could not resolve ${did}`, "unresolved");
  const doc = json as { id?: unknown; alsoKnownAs?: unknown; service?: unknown };
  if (doc.id !== did) throw new IdentityError(`DID document is for another DID`, "mismatch");
  const alsoKnownAs = Array.isArray(doc.alsoKnownAs) ? doc.alsoKnownAs.filter((a): a is string => typeof a === "string") : [];
  const services = Array.isArray(doc.service) ? doc.service : [];
  const pds = services.find(
    (s: { id?: unknown; type?: unknown }) =>
      (s?.id === "#atproto_pds" || s?.id === `${did}#atproto_pds`) && s?.type === "AtprotoPersonalDataServer",
  ) as { serviceEndpoint?: unknown } | undefined;
  if (typeof pds?.serviceEndpoint !== "string") throw new IdentityError(`${did} names no PDS`, "unresolved");
  return { id: did, alsoKnownAs, pds: pds.serviceEndpoint };
}

export interface VerifiedIdentity {
  did: Did;
  handle: string;
  pds: string;
}

/** Handle -> DID -> document, and the document must claim the handle back. */
export async function verifyHandle(handle: string, deps: IdentityDeps): Promise<VerifiedIdentity> {
  const h = normalizeHandle(handle);
  if (!h) throw new IdentityError(`not a valid handle: ${handle}`, "invalid_handle");
  const did = await resolveHandle(h, deps);
  const doc = await resolveDid(did, deps);
  if (!doc.alsoKnownAs.some((a) => a.toLowerCase() === `at://${h}`)) {
    throw new IdentityError(`${did} does not claim the handle ${h}`, "mismatch");
  }
  const pds = await assertPublicUrl(doc.pds, deps);
  return { did, handle: h, pds: pds.origin };
}

/** The DID's verified handle (its first at:// alias that resolves back to it), if any. */
export async function verifiedHandleForDid(did: string, deps: IdentityDeps): Promise<string | undefined> {
  const doc = await resolveDid(did, deps);
  for (const alias of doc.alsoKnownAs) {
    if (!alias.startsWith("at://")) continue;
    const h = normalizeHandle(alias.slice(5));
    if (!h) continue;
    const back = await resolveHandle(h, deps).catch(() => undefined);
    if (back === did) return h;
  }
  return undefined;
}

export async function nodeIdentityDeps(plcUrl: string): Promise<IdentityDeps> {
  const dns = await import("node:dns/promises");
  return {
    fetch,
    resolveTxt: (name) => dns.resolveTxt(name),
    lookup: async (host) => (await dns.lookup(host, { all: true, verbatim: true })).map((a) => a.address),
    plcUrl,
  };
}
