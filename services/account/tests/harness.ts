// The whole app over fakes: a realm, an ATProto network with Alice on it, an OAuth server that
// signs in whoever the test says, and a clock the test moves.

import { currentConsentState } from "../src/oauth.ts";
import { Accounts } from "../src/accounts.ts";
import { createApp, type OAuthPort } from "../src/app.ts";
import { ClientCodes } from "../src/client.ts";
import { Store } from "../src/db.ts";
import { RateLimiter } from "../src/ratelimit.ts";
import { PasswordCipher } from "../src/secrets.ts";
import { WebSessions } from "../src/sessions.ts";
import { addIdentity, ALICE_DID, depsFor, fakeNet, FakeRealm, TEST_KEY } from "./fakes.ts";

export const ORIGIN = "https://wrathplus.artivus.com.au";
export const APP_PASSWORD = "aaaa-bbbb-cccc-dddd";

export function harness(opts: { perIp?: number; perDid?: number } = {}) {
  const net = fakeNet();
  addIdentity(net, { did: ALICE_DID, handle: "alice.bsky.social" });
  const realm = new FakeRealm();
  const store = new Store(":memory:");
  const clock = { now: 1_000_000 };
  const now = () => clock.now;
  const accounts = new Accounts({ store, soap: realm.run, cipher: new PasswordCipher(TEST_KEY), wait: async () => {}, log: () => {} });
  const authorized: Array<{ handle: string; scope: string; state?: string }> = [];
  const oauthState = { scope: "atproto repo:au.com.artivus.wrathplus.character?action=create&action=update repo:au.com.artivus.wrathplus.milestone?action=create&action=update", signOuts: 0 };
  const oauth: OAuthPort = {
    clientMetadata: {},
    jwks: { keys: [] },
    authorize: async (handle, options) => {
      authorized.push({ handle, scope: options.scope, ...(options.state ? { state: options.state } : {}) });
      return new URL("https://auth.example.com/authorize");
    },
    // The fake authorization server returns ?did=…&state=… to the callback.
    callback: async (params) => {
      const did = params.get("did");
      if (!did) throw new Error("access_denied");
      if (!currentConsentState(params.get("state"), store)) throw new Error("Sign-in superseded");
      await store.sessionStore().set(did, { fake: true });
      return { session: { did, getTokenInfo: async () => ({ scope: oauthState.scope }), signOut: async () => { oauthState.signOuts++; await store.sessionStore().del(did); } }, state: params.get("state") };
    },
  };
  const handle = createApp({
    store,
    publicUrl: ORIGIN,
    accounts,
    identity: depsFor(net),
    oauth,
    sessions: new WebSessions(store, TEST_KEY, now),
    codes: new ClientCodes(store, now),
    scope: "atproto repo:au.com.artivus.wrathplus.character?action=create&action=update repo:au.com.artivus.wrathplus.milestone?action=create&action=update",
    realmAddress: "wrathplus.artivus.com.au:3724",
    loginsPerIp: new RateLimiter({ max: opts.perIp ?? 20, windowMs: 60_000 }, now),
    issuesPerDid: new RateLimiter({ max: opts.perDid ?? 10, windowMs: 60_000 }, now),
    log: () => {},
  });

  const get = (path: string, cookie?: string) => handle(new Request(`${ORIGIN}${path}`, { headers: cookie ? { cookie } : {} }), "127.0.0.1");
  const post = (path: string, fields: Record<string, string>, cookie?: string, origin: string | null = ORIGIN) =>
    handle(
      new Request(`${ORIGIN}${path}`, {
        method: "POST",
        headers: { ...(cookie ? { cookie } : {}), ...(origin ? { origin } : {}) },
        body: new URLSearchParams(fields),
      }),
      "127.0.0.1",
    );
  const postJson = (path: string, body: unknown, ip = "198.51.100.7") =>
    handle(
      new Request(`${ORIGIN}${path}`, {
        method: "POST",
        headers: { "content-type": "application/json", "x-forwarded-for": ip },
        body: JSON.stringify(body),
      }),
      "127.0.0.1",
    );

  /** Complete the OAuth round trip as `did`; returns the cookie and the callback response. */
  async function signIn(did = ALICE_DID, state?: string) {
    const q = new URLSearchParams({ did, ...(state ? { state } : {}) });
    const res = await get(`/oauth/callback?${q}`);
    const setCookie = res.headers.get("set-cookie") ?? "";
    return { res, setCookie, cookie: setCookie.split(";")[0]! };
  }

  /** The CSRF token from a page's forms. */
  const csrfOf = async (res: Response) => /name="csrf" value="([^"]+)"/.exec(await res.text())?.[1] ?? "";

  return { oauthState, handle, net, realm, store, clock, authorized, get, post, postJson, signIn, csrfOf };
}
