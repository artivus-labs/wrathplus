// The service's own symmetric keys, generated on first start into one 0600 JSON file:
//   - `password`: AES-256-GCM key for game passwords at rest (one per DID, so "show my account"
//     does not have to reset it);
//   - `cookie`: HMAC-SHA256 key signing the web session cookie.
// Losing the file signs everyone out and makes stored game passwords unreadable; the service
// then issues new ones on the next sign-in (see Accounts.ensure).

import { createCipheriv, createDecipheriv, createHmac, randomBytes, timingSafeEqual } from "node:crypto";
import { closeSync, existsSync, openSync, readFileSync, statSync, writeSync } from "node:fs";

export interface ServiceKeys {
  password: Buffer;
  cookie: Buffer;
}

function decodeKey(value: unknown, name: string, path: string): Buffer {
  const key = typeof value === "string" ? Buffer.from(value, "base64") : Buffer.alloc(0);
  if (key.length !== 32) throw new Error(`${path}: "${name}" must be 32 bytes, base64`);
  return key;
}

export function loadOrCreateServiceKeys(path: string): ServiceKeys {
  if (!existsSync(path)) {
    const body = JSON.stringify({ password: randomBytes(32).toString("base64"), cookie: randomBytes(32).toString("base64") }, null, 2);
    const fd = openSync(path, "wx", 0o600); // O_EXCL: never overwrite a key
    try {
      writeSync(fd, body);
    } finally {
      closeSync(fd);
    }
  }
  if (statSync(path).mode & 0o077) throw new Error(`${path} must not be readable by group or others (chmod 600)`);
  const json = JSON.parse(readFileSync(path, "utf8")) as Record<string, unknown>;
  return { password: decodeKey(json.password, "password", path), cookie: decodeKey(json.cookie, "cookie", path) };
}

/** AES-256-GCM with the DID as associated data: a ciphertext only opens for the DID it was made for. */
export class PasswordCipher {
  constructor(private readonly key: Buffer) {
    if (key.length !== 32) throw new Error("AES-256-GCM needs a 32-byte key");
  }

  encrypt(did: string, password: string): string {
    const iv = randomBytes(12);
    const cipher = createCipheriv("aes-256-gcm", this.key, iv);
    cipher.setAAD(Buffer.from(did, "utf8"));
    const body = Buffer.concat([cipher.update(password, "utf8"), cipher.final()]);
    return `v1:${Buffer.concat([iv, body, cipher.getAuthTag()]).toString("base64url")}`;
  }

  /** The password, or undefined if the blob is malformed, for another DID, or under another key. */
  decrypt(did: string, blob: string): string | undefined {
    if (!blob.startsWith("v1:")) return undefined;
    const raw = Buffer.from(blob.slice(3), "base64url");
    if (raw.length < 12 + 16 + 1) return undefined;
    try {
      const decipher = createDecipheriv("aes-256-gcm", this.key, raw.subarray(0, 12));
      decipher.setAAD(Buffer.from(did, "utf8"));
      decipher.setAuthTag(raw.subarray(raw.length - 16));
      return Buffer.concat([decipher.update(raw.subarray(12, raw.length - 16)), decipher.final()]).toString("utf8");
    } catch {
      return undefined;
    }
  }
}

export function hmac(key: Buffer, value: string): string {
  return createHmac("sha256", key).update(value).digest("base64url");
}

export function safeEqual(a: string, b: string): boolean {
  const x = Buffer.from(a);
  const y = Buffer.from(b);
  return x.length === y.length && timingSafeEqual(x, y);
}

export function randomToken(bytes = 32): string {
  return randomBytes(bytes).toString("base64url");
}
