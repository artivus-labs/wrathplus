// Configuration from the environment. Secrets are never in the environment: the SOAP account
// comes from a 0600 file named by SOAP_CREDENTIALS_FILE.

import { join } from "node:path";
import { DEFAULT_HOSTED_SUFFIXES, DEFAULT_RESERVED_PREFIXES } from "./naming.ts";

export interface Config {
  publicUrl: string;
  host: string;
  port: number;
  dataDir: string;
  dbPath: string;
  keysPath: string;
  serviceKeysPath: string;
  soapUrl: string;
  soapCredentialsFile: string;
  realmAddress: string;
  realmName: string;
  plcUrl: string;
  eventPollMs: number;
  publishQuests: boolean;
  reservedPrefixes: string[];
  hostedSuffixes: string[];
  loginsPerIp: { max: number; windowMs: number };
  issuesPerDid: { max: number; windowMs: number };
}

type Env = Record<string, string | undefined>;

function int(env: Env, key: string, fallback: number): number {
  const raw = env[key];
  if (raw === undefined || raw === "") return fallback;
  const n = Number(raw);
  if (!Number.isSafeInteger(n) || n < 0) throw new Error(`${key} must be a non-negative integer`);
  return n;
}

function list(env: Env, key: string, fallback: readonly string[]): string[] {
  const raw = env[key];
  if (raw === undefined) return [...fallback];
  return raw.split(",").map((s) => s.trim()).filter(Boolean);
}

export function loadConfig(env: Env = process.env): Config {
  const dataDir = env.ACCOUNT_DATA_DIR || "./data";
  const soapCredentialsFile = env.SOAP_CREDENTIALS_FILE;
  if (!soapCredentialsFile) throw new Error("SOAP_CREDENTIALS_FILE is required (a 0600 file holding USER:PASSWORD)");
  const publicUrl = (env.PUBLIC_URL || "https://wrathplus.artivus.com.au").replace(/\/$/, "");
  if (!publicUrl.startsWith("https://")) throw new Error("PUBLIC_URL must be https (ATProto OAuth confidential clients require it)");
  const host = env.LISTEN_HOST || "127.0.0.1";
  return {
    publicUrl,
    host,
    port: int(env, "LISTEN_PORT", 8787),
    dataDir,
    dbPath: join(dataDir, "account.sqlite"),
    keysPath: join(dataDir, "oauth-keys.json"),
    serviceKeysPath: join(dataDir, "service-keys.json"),
    soapUrl: env.SOAP_URL || "http://127.0.0.1:17878/",
    soapCredentialsFile,
    realmAddress: env.REALM_ADDRESS || "wrathplus.artivus.com.au:3724",
    realmName: env.REALM_NAME || "wrathplus.artivus.com.au",
    plcUrl: env.PLC_URL || "https://plc.directory",
    eventPollMs: int(env, "EVENT_POLL_MS", 5000),
    publishQuests: (env.PUBLISH_QUESTS ?? "0") === "1",
    reservedPrefixes: list(env, "RESERVED_ACCOUNT_PREFIXES", DEFAULT_RESERVED_PREFIXES).map((p) => p.toUpperCase()),
    hostedSuffixes: list(env, "HOSTED_HANDLE_SUFFIXES", DEFAULT_HOSTED_SUFFIXES),
    loginsPerIp: { max: int(env, "LOGINS_PER_IP", 20), windowMs: int(env, "LOGINS_PER_IP_WINDOW_S", 600) * 1000 },
    issuesPerDid: { max: int(env, "ISSUES_PER_DID", 10), windowMs: int(env, "ISSUES_PER_DID_WINDOW_S", 3600) * 1000 },
  };
}
