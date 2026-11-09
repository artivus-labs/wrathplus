// WrathPlus account service: wiring and the event poll loop.

import { Accounts } from "./accounts.ts";
import { createApp } from "./app.ts";
import { loadConfig } from "./config.ts";
import { Store } from "./db.ts";
import { nodeIdentityDeps } from "./identity.ts";
import { createOAuthClient, loadOrCreateKeys, OAUTH_SCOPE } from "./oauth.ts";
import { RateLimiter } from "./ratelimit.ts";
import { EventPump, oauthPublisher } from "./records.ts";
import { readSoapCredentials, soapClient } from "./soap.ts";

process.umask(0o077);
const config = loadConfig();
const store = new Store(config.dbPath);
const keys = await loadOrCreateKeys(config.keysPath);
const identity = await nodeIdentityDeps(config.plcUrl);
const oauth = createOAuthClient(config.publicUrl, keys, store, identity);
// Read per call so a rotated credential file takes effect without a restart.
const soap = soapClient(config.soapUrl, () => readSoapCredentials(config.soapCredentialsFile));
readSoapCredentials(config.soapCredentialsFile); // fail fast on a missing or world-readable file

const accounts = new Accounts({
  store,
  soap,
  naming: { reservedPrefixes: config.reservedPrefixes, hostedSuffixes: config.hostedSuffixes },
});

const app = createApp({
  accounts,
  identity,
  oauth,
  scope: OAUTH_SCOPE,
  realmAddress: config.realmAddress,
  loginsPerIp: new RateLimiter(config.loginsPerIp),
  issuesPerDid: new RateLimiter(config.issuesPerDid),
});

const server = Bun.serve({
  hostname: config.host,
  port: config.port,
  maxRequestBodySize: 16 * 1024,
  fetch: (req, srv) => app(req, srv.requestIP(req)?.address),
});
console.log(`account service on http://${server.hostname}:${server.port} for ${config.publicUrl}`);

if (config.eventPollMs > 0) {
  const pump = new EventPump({
    soap,
    publish: oauthPublisher((did) => oauth.restore(did), (did) => store.hasSession(did)),
    realm: config.realmName,
    publishQuests: config.publishQuests,
  });
  let quietUntil = 0;
  const loop = async () => {
    try {
      // Drain a backlog quickly, then idle at the poll interval.
      while ((await pump.tick()).handled > 0);
    } catch (e) {
      const now = Date.now();
      if (now > quietUntil) {
        console.log(`records: realm poll failed: ${(e as Error).message}`);
        quietUntil = now + 60_000;
      }
    }
    setTimeout(loop, config.eventPollMs);
  };
  setTimeout(loop, config.eventPollMs);
}

for (const signal of ["SIGINT", "SIGTERM"] as const) {
  process.on(signal, () => {
    server.stop();
    store.close();
    process.exit(0);
  });
}
