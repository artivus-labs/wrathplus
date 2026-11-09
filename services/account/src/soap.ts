// The realm's SOAP command port (AzerothCore worldserver, `SOAP.Enabled`), loopback only.
//
// Commands are built only from validated tokens: account names and passwords are [A-Z0-9], DIDs
// and handles pass the ATProto syntax checks (letters, digits, '.', '-', ':'). Nothing a user
// typed reaches a command otherwise, and no token can contain a space (the realm splits
// arguments on spaces) or a newline. The XML envelope escapes the command as well.

import { readFileSync, statSync } from "node:fs";
import { ACCOUNT_NAME } from "./naming.ts";
import { GAME_PASSWORD } from "./password.ts";
import { isDid, normalizeHandle } from "./identity.ts";

export class SoapError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
  get alreadyExists(): boolean {
    return /already exist/i.test(this.message);
  }
  get notExists(): boolean {
    return /not exist/i.test(this.message);
  }
  get unknownCommand(): boolean {
    return /no such (sub)?command/i.test(this.message);
  }
}

export type SoapRun = (command: string) => Promise<string>;

function accountToken(name: string): string {
  if (!ACCOUNT_NAME.test(name)) throw new Error("invalid account name for a realm command");
  return name;
}
function passwordToken(password: string): string {
  if (!GAME_PASSWORD.test(password)) throw new Error("invalid game password for a realm command");
  return password;
}
function didToken(did: string): string {
  if (!isDid(did)) throw new Error("invalid DID for a realm command");
  return did;
}
function handleToken(handle: string): string {
  const h = normalizeHandle(handle);
  if (!h || h !== handle) throw new Error("invalid handle for a realm command");
  return h;
}
function countToken(n: number, max: number): string {
  if (!Number.isSafeInteger(n) || n < 0 || n > max) throw new Error("invalid number for a realm command");
  return String(n);
}

export const commands = {
  createAccount: (name: string, password: string) => `.account create ${accountToken(name)} ${passwordToken(password)}`,
  setPassword: (name: string, password: string) =>
    `.account set password ${accountToken(name)} ${passwordToken(password)} ${passwordToken(password)}`,
  /** Expansion 2 (Wrath of the Lich King). */
  setAddon: (name: string) => `.account set addon ${accountToken(name)} 2`,
  link: (name: string, did: string, handle: string) => `.atproto link ${accountToken(name)} ${didToken(did)} ${handleToken(handle)}`,
  events: (limit: number) => `.atproto events ${countToken(limit, 500)}`,
  ack: (id: number) => `.atproto ack ${countToken(id, Number.MAX_SAFE_INTEGER)}`,
} as const;

export function escapeXml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&apos;");
}

export function unescapeXml(s: string): string {
  return s
    .replace(/&#x([0-9a-fA-F]+);/g, (_, h: string) => String.fromCodePoint(parseInt(h, 16)))
    .replace(/&#([0-9]+);/g, (_, d: string) => String.fromCodePoint(parseInt(d, 10)))
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&amp;/g, "&");
}

export function envelope(command: string): string {
  if (/[\r\n\0]/.test(command)) throw new Error("realm commands are one line");
  return (
    '<?xml version="1.0" encoding="utf-8"?>' +
    '<SOAP-ENV:Envelope xmlns:SOAP-ENV="http://schemas.xmlsoap.org/soap/envelope/" xmlns:ns1="urn:AC">' +
    `<SOAP-ENV:Body><ns1:executeCommand><command>${escapeXml(command)}</command></ns1:executeCommand></SOAP-ENV:Body>` +
    "</SOAP-ENV:Envelope>"
  );
}

function between(text: string, open: string, close: string): string | undefined {
  const start = text.indexOf(open);
  if (start < 0) return undefined;
  const end = text.indexOf(close, start + open.length);
  if (end < 0) return undefined;
  return unescapeXml(text.slice(start + open.length, end)).trim();
}

/** The command's output, or a SoapError carrying the realm's message. */
export function parseSoapResponse(status: number, body: string): string {
  if (status === 200) return between(body, "<result>", "</result>") ?? "";
  if (status === 401 || status === 403) throw new SoapError("the realm refused the SOAP account", status);
  throw new SoapError(between(body, "<faultstring>", "</faultstring>") || `SOAP command failed (HTTP ${status})`, status);
}

export interface SoapCredentials {
  user: string;
  password: string;
}

/** `USER:PASSWORD` on the first line of a file only its owner can read. */
export function readSoapCredentials(path: string): SoapCredentials {
  const mode = statSync(path).mode & 0o777;
  if (mode & 0o077) throw new Error(`${path} must not be readable by group or others (chmod 600)`);
  const line = readFileSync(path, "utf8").split("\n")[0]?.trim() ?? "";
  const at = line.indexOf(":");
  if (at <= 0 || at === line.length - 1) throw new Error(`${path} must hold USER:PASSWORD`);
  return { user: line.slice(0, at), password: line.slice(at + 1) };
}

export function soapClient(url: string, credentials: () => SoapCredentials, fetchImpl: typeof fetch = fetch): SoapRun {
  return async (command) => {
    const { user, password } = credentials();
    const res = await fetchImpl(url, {
      method: "POST",
      headers: {
        "content-type": "text/xml; charset=utf-8",
        authorization: `Basic ${Buffer.from(`${user}:${password}`).toString("base64")}`,
      },
      body: envelope(command),
      signal: AbortSignal.timeout(30_000),
    });
    return parseSoapResponse(res.status, await res.text());
  };
}
