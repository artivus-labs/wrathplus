// Test doubles: a realm that answers SOAP commands like AzerothCore, and an ATProto network
// (DNS, PLC directory, PDS) made of plain maps.

import type { IdentityDeps } from "../src/identity.ts";
import { SoapError } from "../src/soap.ts";

export class FakeRealm {
  readonly accounts = new Map<string, { password: string; addon: number }>();
  readonly links = new Map<string, { did: string; handle: string }>();
  readonly commands: string[] = [];
  /** Commands on an account report "does not exist" this many times after it is created. */
  lag = 0;
  private pending = new Map<string, number>();
  moduleInstalled = true;
  events: string[] = [];
  acked: number[] = [];

  run = async (command: string): Promise<string> => {
    this.commands.push(command);
    const parts = command.split(" ");
    const [verb, sub, a, b, c] = parts;
    const missing = (name: string) => {
      const left = this.pending.get(name) ?? 0;
      if (left > 0) {
        this.pending.set(name, left - 1);
        return true;
      }
      return !this.accounts.has(name);
    };
    if (verb === ".account" && sub === "create") {
      if (this.accounts.has(a!)) throw new SoapError("Account already exist!", 500);
      this.accounts.set(a!, { password: b!, addon: 0 });
      this.pending.set(a!, this.lag);
      return `Account created: ${a}`;
    }
    if (verb === ".account" && sub === "set" && a === "password") {
      if (!this.accounts.has(b!)) throw new SoapError(`Account ${b} does not exist`, 500);
      if (parts[4] !== parts[5]) throw new SoapError("passwords do not match", 500);
      this.accounts.get(b!)!.password = c!;
      return "The password was changed";
    }
    if (verb === ".account" && sub === "set" && a === "addon") {
      if (missing(b!)) throw new SoapError(`Account ${b} does not exist`, 500);
      this.accounts.get(b!)!.addon = Number(c);
      return "ok";
    }
    if (verb === ".atproto") {
      if (!this.moduleInstalled) throw new SoapError("There is no such command.", 500);
      if (sub === "link") {
        if (missing(a!)) throw new SoapError(`Account ${a} does not exist`, 500);
        this.links.set(a!, { did: b!, handle: c! });
        return `Linked ${a}`;
      }
      if (sub === "events") return this.events.slice(0, Number(a)).join("\n");
      if (sub === "ack") {
        const id = Number(a);
        this.acked.push(id);
        this.events = this.events.filter((e) => Number(e.split(" ")[1]) > id);
        return "ok";
      }
    }
    throw new SoapError("There is no such command.", 500);
  };
}

export interface FakeNet {
  txt: Record<string, string[]>;
  hosts: Record<string, string[]>;
  routes: Record<string, (req: Request) => Response | Promise<Response>>;
  requests: Request[];
}

export function fakeNet(): FakeNet {
  return { txt: {}, hosts: {}, routes: {}, requests: [] };
}

export function depsFor(net: FakeNet): IdentityDeps {
  return {
    plcUrl: "https://plc.directory",
    resolveTxt: async (name) => {
      const r = net.txt[name];
      if (!r) throw Object.assign(new Error("ENOTFOUND"), { code: "ENOTFOUND" });
      return r.map((t) => [t]);
    },
    lookup: async (host) => net.hosts[host] ?? [],
    fetch: (async (input: string | URL | Request, init?: RequestInit) => {
      const req = (input instanceof Request ? new Request(input, init) : new Request(input.toString(), init));
      net.requests.push(req);
      const u = new URL(req.url);
      const route = net.routes[`${req.method} ${u.origin}${u.pathname}`];
      if (!route) return new Response("not found", { status: 404 });
      return route(req);
    }) as typeof fetch,
  };
}

export const ALICE_DID = "did:plc:abcdefghijklmnopqrstuvwx";
export const BOB_DID = "did:plc:bbbbbbbbbbbbbbbbbbbbbbbb";

export function didDoc(did: string, handle: string, pds: string) {
  return {
    id: did,
    alsoKnownAs: [`at://${handle}`],
    service: [{ id: "#atproto_pds", type: "AtprotoPersonalDataServer", serviceEndpoint: pds }],
  };
}

export function jwt(payload: Record<string, unknown>): string {
  const b = (o: unknown) => Buffer.from(JSON.stringify(o)).toString("base64url");
  return `${b({ alg: "ES256K" })}.${b(payload)}.sig`;
}

/** A handle on a PDS, resolvable by DNS, with an app password. */
export function addIdentity(net: FakeNet, opts: { did: string; handle: string; pds?: string; appPassword?: string; scope?: string; sessionDid?: string }) {
  const pds = opts.pds ?? "https://pds.example.com";
  net.txt[`_atproto.${opts.handle}`] = [`did=${opts.did}`];
  net.hosts["plc.directory"] = ["104.16.0.1"];
  net.hosts[new URL(pds).hostname] ??= ["203.0.113.10"];
  net.routes[`GET https://plc.directory/${opts.did}`] = () => Response.json(didDoc(opts.did, opts.handle, pds));
  net.routes[`POST ${pds}/xrpc/com.atproto.server.createSession`] = async (req) => {
    const body = (await req.json()) as { identifier: string; password: string };
    if (body.password !== (opts.appPassword ?? "aaaa-bbbb-cccc-dddd")) {
      return Response.json({ error: "AuthenticationRequired" }, { status: 401 });
    }
    return Response.json({
      did: opts.sessionDid ?? opts.did,
      handle: opts.handle,
      accessJwt: jwt({ scope: opts.scope ?? "com.atproto.appPass", sub: opts.did }),
      refreshJwt: jwt({ scope: "com.atproto.refresh" }),
    });
  };
  net.routes[`POST ${pds}/xrpc/com.atproto.server.deleteSession`] = () => new Response(null, { status: 200 });
}

export const TEST_KEY = Buffer.alloc(32, 7);
