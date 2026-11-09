// Agent sign-in with an ATProto app password, used only to prove who the agent is:
//   1. resolve the handle to a DID and that DID's PDS (both directions verified);
//   2. com.atproto.server.createSession on that PDS with the app password;
//   3. the session's DID must be the resolved DID;
//   4. revoke the session at once (deleteSession) and forget the tokens and the password.
// We never keep a write credential for an agent, so its records are its own business (it can
// publish them with its own session; see the README).

import { guardedJson, IdentityError, normalizeHandle, verifyHandle, type IdentityDeps } from "./identity.ts";

export class AgentLoginError extends Error {
  constructor(
    message: string,
    readonly status: 400 | 401 | 403 | 502,
  ) {
    super(message);
  }
}

export interface ProvenAgent {
  did: string;
  handle: string;
}

const MAX_PASSWORD = 256;

function jwtScope(token: unknown): string | undefined {
  if (typeof token !== "string") return undefined;
  const part = token.split(".")[1];
  if (!part) return undefined;
  try {
    const payload = JSON.parse(Buffer.from(part, "base64url").toString("utf8")) as { scope?: unknown };
    return typeof payload.scope === "string" ? payload.scope : undefined;
  } catch {
    return undefined;
  }
}

export async function proveAgentIdentity(handleInput: unknown, appPassword: unknown, deps: IdentityDeps): Promise<ProvenAgent> {
  if (typeof handleInput !== "string" || typeof appPassword !== "string") {
    throw new AgentLoginError("handle and app_password are required strings", 400);
  }
  const handle = normalizeHandle(handleInput);
  if (!handle) throw new AgentLoginError("not a valid handle", 400);
  if (appPassword.length === 0 || appPassword.length > MAX_PASSWORD) throw new AgentLoginError("not a valid app password", 400);

  let identity;
  try {
    identity = await verifyHandle(handle, deps);
  } catch (e) {
    if (e instanceof IdentityError) {
      throw new AgentLoginError(`could not verify ${handle}: ${e.message}`, e.code === "invalid_handle" ? 400 : 502);
    }
    throw e;
  }

  const createSession = `${identity.pds}/xrpc/com.atproto.server.createSession`;
  let res;
  try {
    res = await guardedJson(createSession, deps, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ identifier: identity.handle, password: appPassword }),
    });
  } catch {
    throw new AgentLoginError("your PDS did not answer", 502);
  }
  if (res.status === 401 || res.status === 400) throw new AgentLoginError("the PDS rejected the handle or app password", 401);
  if (res.status !== 200 || typeof res.json !== "object" || res.json === null) throw new AgentLoginError(`your PDS answered ${res.status}`, 502);
  const session = res.json as { did?: unknown; accessJwt?: unknown; refreshJwt?: unknown };

  // Revoke what we were just given, whatever happens next.
  if (typeof session.refreshJwt === "string") {
    guardedJson(`${identity.pds}/xrpc/com.atproto.server.deleteSession`, deps, {
      method: "POST",
      headers: { authorization: `Bearer ${session.refreshJwt}` },
    }).catch(() => undefined);
  }

  if (session.did !== identity.did) throw new AgentLoginError("the PDS signed in a different account than the handle names", 403);
  // A full account password signs in too; ask for an app password so nobody hands us their main one.
  if (jwtScope(session.accessJwt) === "com.atproto.access") {
    throw new AgentLoginError("use an app password (Settings > Privacy and security > App passwords), not your account password", 401);
  }
  return { did: identity.did, handle: identity.handle };
}
