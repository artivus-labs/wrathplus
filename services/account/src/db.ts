// The service's own state, one SQLite file: DID -> game account links, and the OAuth library's
// state (in-flight logins) and session (refresh tokens, DPoP keys) stores. The file is created
// 0600 in a 0700 directory: sessions are write credentials for people's repositories.

import { Database } from "bun:sqlite";
import { chmodSync, existsSync, mkdirSync } from "node:fs";
import { dirname } from "node:path";

export interface AccountLink {
  did: string;
  account: string;
  handle: string;
  createdAt: number;
  updatedAt: number;
  /** The game password, encrypted (PasswordCipher); absent for links made before v2. */
  passwordEnc?: string;
}

export interface KeyValueStore<V> {
  get(key: string): Promise<V | undefined>;
  set(key: string, value: V): Promise<void>;
  del(key: string): Promise<void>;
}

export class Store {
  readonly db: Database;

  constructor(path: string) {
    if (path !== ":memory:") {
      const dir = dirname(path);
      if (!existsSync(dir)) mkdirSync(dir, { recursive: true, mode: 0o700 });
      const fresh = !existsSync(path);
      this.db = new Database(path, { create: true, strict: true });
      if (fresh) chmodSync(path, 0o600);
    } else {
      this.db = new Database(":memory:", { strict: true });
    }
    this.db.exec("PRAGMA journal_mode = WAL");
    this.db.exec(`
      CREATE TABLE IF NOT EXISTS account_link (
        did TEXT PRIMARY KEY,
        account TEXT NOT NULL UNIQUE,
        handle TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
      );
      CREATE TABLE IF NOT EXISTS oauth_state (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL,
        expires_at INTEGER NOT NULL
      );
      CREATE TABLE IF NOT EXISTS oauth_session (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL,
        updated_at INTEGER NOT NULL
      );
      CREATE TABLE IF NOT EXISTS web_session (
        id_hash TEXT PRIMARY KEY,
        did TEXT NOT NULL,
        handle TEXT NOT NULL,
        csrf TEXT NOT NULL,
        expires_at INTEGER NOT NULL
      );
      CREATE TABLE IF NOT EXISTS client_code (
        code_hash TEXT PRIMARY KEY,
        did TEXT NOT NULL,
        handle TEXT NOT NULL,
        challenge TEXT NOT NULL,
        redirect TEXT NOT NULL,
        expires_at INTEGER NOT NULL
      );
    `);
    // v2: the game password, AES-256-GCM (see secrets.ts). Added to databases made by v1.
    const columns = this.db.query<{ name: string }, []>("PRAGMA table_info(account_link)").all();
    if (!columns.some((c) => c.name === "password_enc")) this.db.exec("ALTER TABLE account_link ADD COLUMN password_enc TEXT");
  }

  linkByDid(did: string): AccountLink | undefined {
    const row = this.db
      .query<{ did: string; account: string; handle: string; created_at: number; updated_at: number; password_enc: string | null }, [string]>(
        "SELECT did, account, handle, created_at, updated_at, password_enc FROM account_link WHERE did = ?",
      )
      .get(did);
    return row
      ? {
          did: row.did,
          account: row.account,
          handle: row.handle,
          createdAt: row.created_at,
          updatedAt: row.updated_at,
          ...(row.password_enc ? { passwordEnc: row.password_enc } : {}),
        }
      : undefined;
  }

  accountTaken(account: string): boolean {
    return this.db.query<{ n: number }, [string]>("SELECT 1 AS n FROM account_link WHERE account = ?").get(account) !== null;
  }

  insertLink(did: string, account: string, handle: string, passwordEnc: string, now = Date.now()): void {
    this.db
      .query("INSERT INTO account_link (did, account, handle, created_at, updated_at, password_enc) VALUES (?, ?, ?, ?, ?, ?)")
      .run(did, account, handle, now, now, passwordEnc);
  }

  touchLink(did: string, handle: string, now = Date.now()): void {
    this.db.query("UPDATE account_link SET handle = ?, updated_at = ? WHERE did = ?").run(handle, now, did);
  }

  setPassword(did: string, passwordEnc: string, now = Date.now()): void {
    this.db.query("UPDATE account_link SET password_enc = ?, updated_at = ? WHERE did = ?").run(passwordEnc, now, did);
  }

  // --- web sessions (ids and codes are stored hashed) ---------------------------------------------

  insertWebSession(idHash: string, did: string, handle: string, csrf: string, expiresAt: number, now = Date.now()): void {
    this.db.query("DELETE FROM web_session WHERE expires_at < ?").run(now);
    this.db.query("INSERT INTO web_session (id_hash, did, handle, csrf, expires_at) VALUES (?, ?, ?, ?, ?)").run(idHash, did, handle, csrf, expiresAt);
  }

  webSession(idHash: string): { did: string; handle: string; csrf: string; expiresAt: number } | undefined {
    const row = this.db
      .query<{ did: string; handle: string; csrf: string; expires_at: number }, [string]>("SELECT did, handle, csrf, expires_at FROM web_session WHERE id_hash = ?")
      .get(idHash);
    return row ? { did: row.did, handle: row.handle, csrf: row.csrf, expiresAt: row.expires_at } : undefined;
  }

  deleteWebSession(idHash: string): void {
    this.db.query("DELETE FROM web_session WHERE id_hash = ?").run(idHash);
  }

  insertClientCode(codeHash: string, did: string, handle: string, challenge: string, redirect: string, expiresAt: number, now = Date.now()): void {
    this.db.query("DELETE FROM client_code WHERE expires_at < ?").run(now);
    this.db
      .query("INSERT INTO client_code (code_hash, did, handle, challenge, redirect, expires_at) VALUES (?, ?, ?, ?, ?, ?)")
      .run(codeHash, did, handle, challenge, redirect, expiresAt);
  }

  /** Remove and return a code in one statement: a code can be taken once, whatever happens next. */
  takeClientCode(codeHash: string): { did: string; handle: string; challenge: string; redirect: string; expiresAt: number } | undefined {
    const row = this.db
      .query<{ did: string; handle: string; challenge: string; redirect: string; expires_at: number }, [string]>(
        "DELETE FROM client_code WHERE code_hash = ? RETURNING did, handle, challenge, redirect, expires_at",
      )
      .get(codeHash);
    return row ? { did: row.did, handle: row.handle, challenge: row.challenge, redirect: row.redirect, expiresAt: row.expires_at } : undefined;
  }

  /** The OAuth library's in-flight login state; entries older than an hour are dropped. */
  stateStore<V>(ttlMs = 60 * 60 * 1000, now = () => Date.now()): KeyValueStore<V> {
    return {
      get: async (key) => {
        const row = this.db.query<{ value: string; expires_at: number }, [string]>("SELECT value, expires_at FROM oauth_state WHERE key = ?").get(key);
        if (!row || row.expires_at < now()) return undefined;
        return JSON.parse(row.value) as V;
      },
      set: async (key, value) => {
        this.db.query("DELETE FROM oauth_state WHERE expires_at < ?").run(now());
        this.db.query("INSERT OR REPLACE INTO oauth_state (key, value, expires_at) VALUES (?, ?, ?)").run(key, JSON.stringify(value), now() + ttlMs);
      },
      del: async (key) => {
        this.db.query("DELETE FROM oauth_state WHERE key = ?").run(key);
      },
    };
  }

  /** The OAuth library's sessions, keyed by DID. */
  sessionStore<V>(): KeyValueStore<V> {
    return {
      get: async (key) => {
        const row = this.db.query<{ value: string }, [string]>("SELECT value FROM oauth_session WHERE key = ?").get(key);
        return row ? (JSON.parse(row.value) as V) : undefined;
      },
      set: async (key, value) => {
        this.db.query("INSERT OR REPLACE INTO oauth_session (key, value, updated_at) VALUES (?, ?, ?)").run(key, JSON.stringify(value), Date.now());
      },
      del: async (key) => {
        this.db.query("DELETE FROM oauth_session WHERE key = ?").run(key);
      },
    };
  }

  hasSession(did: string): boolean {
    return this.db.query("SELECT 1 FROM oauth_session WHERE key = ?").get(did) !== null;
  }

  close(): void {
    this.db.close();
  }
}
