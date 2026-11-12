# WrathPlus account service

Sign in with ATProto, get a game account on the beta realm (`wrathplus.artivus.com.au`).

- **People** sign in with ATProto OAuth on the web page: a confidential client
  (`@atproto/oauth-client-node`, `private_key_jwt` with ES256, PAR, PKCE and DPoP). The client
  metadata is published at `https://wrathplus.artivus.com.au/oauth/client-metadata.json`.
  Afterwards they have a web session and see their account and password on the home page.
- **The WrathPlus client and launcher** sign in through the browser with a loopback redirect and
  PKCE: see [CLIENT-LOGIN.md](CLIENT-LOGIN.md).
- **Agents** sign in with an app password: `POST /api/agent/login`.

Either way the identity is proven first; the DID is the permanent key. The service makes the DID's
one game account on the realm over SOAP the first time, with a random password, and keeps that
password encrypted at rest (AES-256-GCM, bound to the DID). Signing in again, viewing the account
or signing in the client returns the same password. It changes only on an explicit reset: the
**Reset password** button (a POST with the session's CSRF token, same-origin only) or
`"reset": true` from an agent.

```sh
curl -s https://wrathplus.artivus.com.au/api/agent/login \
  -H 'content-type: application/json' \
  -d '{"handle":"mybot.bsky.social","app_password":"xxxx-xxxx-xxxx-xxxx"}'
# {"account":"MYBOT","password":"K7Q2…","realm":"wrathplus.artivus.com.au:3724","did":"did:plc:…","handle":"mybot.bsky.social"}
# add "reset": true for a new password
```

Then point a 3.3.5a client (or a WrathPlus agent) at `set realmlist wrathplus.artivus.com.au`
and log in with `account` / `password`. Passwords are not case-sensitive in game. A password
changed in game (`.account password`) is not seen by the service; reset it on the website to get
a known one again.

## Web sessions

After OAuth the browser gets `__Host-wp_session`: a random id plus its HMAC-SHA256, `HttpOnly`,
`Secure`, `SameSite=Lax`, `Path=/`, 30 days. The database keeps only the id's hash, the DID, the
verified handle and the session's CSRF token, so signing out ends the session server-side.
State-changing forms (reset, sign out, client approval) need that CSRF token and refuse a
cross-origin `Origin` header.

## How identity is checked

- Handle → DID by DNS (`_atproto.<handle>` TXT) then `https://<handle>/.well-known/atproto-did`;
  DID → document (PLC directory or `did:web`); the document must list `at://<handle>` back.
- **Agents:** `com.atproto.server.createSession` on the PDS named in that document. The session's
  DID must equal the resolved DID. The session is deleted at once; the app password and tokens
  are never stored or logged, and never sent to the realm. A main account password (session scope
  `com.atproto.access`) is refused: use an app password.
- **People:** the OAuth library binds the token to the DID (`sub`); the handle shown is the DID's
  `alsoKnownAs` handle, verified back to the DID. A DID whose handle does not verify is refused.
- Every URL fetched on someone else's say-so (PDS, authorization server, did:web, well-known) must
  be https on a host that resolves only to public addresses: the service sits next to the realm's
  SOAP port. (The library's own guard needs Node's undici and fails on Bun, so ours replaces it.
  It checks DNS before connecting; a rebinding race between that check and the connection is not
  closed.)

## Game account names

At most 16 characters, `A-Z0-9`. The first choice is the handle without dots or hyphens, with a
shared hosting suffix (`bsky.social`) dropped: `alice.bsky.social` → `ALICE`,
`bob.example.com` → `BOBEXAMPLECOM`, cut to 16. If that name is taken (by another DID, or by any
realm account the service did not create: bots, admins), the next candidates are the first 11
characters plus 5 characters of `base32(sha256(DID#k))`, so collisions resolve the same way every
time for the same identity. Names starting with a reserved prefix (`RNDBOT`: mod-playerbots counts
`username LIKE 'RNDBOT%'` as its own; `AHBOT`, `WPMANAGER`, `ADMIN`, `SOAP`) get `AT` in front.
The mapping is stored by DID and never changes, even when the handle does.

## Privacy and optional records

New and existing accounts default to private. Website sign-in requests only identity scope
(`atproto`) unless the unchecked public-record option is explicitly selected. Consent is
kept privately and checked after OAuth restore and before every record write. Old stored grants
are discarded on migration; DID/account/password continuity is preserved. Private callbacks
end their temporary OAuth session and remove locally stored credentials.

The account page can start opt-in authorization or stop future publication (CSRF-protected).
Withdrawal removes local OAuth credentials and fences stale in-flight opt-in callbacks. Queued
private events are acknowledged and dropped, not backfilled after later opt-in. A network write
already dispatched before withdrawal cannot be recalled. Existing public records are not
automatically deleted; public copies may persist after manual deletion. Private server data
(DID/handle/account mapping, encrypted password and gameplay) remains necessary to operate
the account. Other players still see ordinary in-game character activity.


Lexicons (in `lexicons/`, authority `wrathplus.artivus.com.au`):

- `au.com.artivus.wrathplus.character`: one record per character, key `<realm>:<guid>`.
- `au.com.artivus.wrathplus.milestone`: a level reached, an achievement earned, a quest turned in
  (quests only with `PUBLISH_QUESTS=1`), key `<realm>:<guid>:<kind>:<value>` (quests add the time).

For explicit opt-in only, the OAuth scope is `atproto` plus `repo:<collection>?action=create&action=update` for these two
collections only. The service polls the realm module (`mod-atproto-identity`, `.atproto events`
over SOAP), writes the records with the person's stored OAuth session, then acknowledges them.
Record keys come from the event's content, so a repeat delivery overwrites instead of duplicating.

**Agent app-password sign-in** never enables publication or retains its proof session.
Identities without explicit website consent publish nothing; their events are acknowledged and
dropped. An existing website opt-in is unchanged by agent sign-in and can be disabled on the
account page. An agent that wants
records writes them itself with its own app-password session (`com.atproto.repo.putRecord`, same
collections and keys).

The schemas are published from the account that owns the NSID authority (Vulcan's Delve Town
identity, Skein). `scripts/publish-lexicons.ts` signs in with that account's app password (from
`LEXICON_PUBLISH_CREDENTIALS_FILE`, a 0600 file holding `handle:app-password`, or
`LEXICON_PUBLISH_HANDLE` and `LEXICON_PUBLISH_APP_PASSWORD`; never argv), puts each schema as a
`com.atproto.lexicon.schema` record with key = NSID, signs out, and prints the DNS record to
create, `_lexicon.wrathplus.artivus.com.au TXT "did=<DID>"`. It does not create the record.
`--dry-run` only validates and lists the schemas. Fields may only be added to a published
schema, and only as optional ones.

## Run

Needs Bun 1.4 or newer (1.2 cannot load the OAuth library's undici dependency).

```sh
bun install --frozen-lockfile
bun test
bun run typecheck
SOAP_CREDENTIALS_FILE=/etc/wrathplus/soap-account bun run src/index.ts
```

Configuration is environment only (`account.env.example`); the SOAP account comes from a file
holding `USER:PASSWORD`, which must be mode 0600 (the service refuses a readable one). State in
`ACCOUNT_DATA_DIR`, all created 0600 on first start: one SQLite file; `oauth-keys.json` (OAuth
signing keys: losing them stops record publishing until people sign in again); `service-keys.json`
(the AES-256-GCM key for stored game passwords and the cookie HMAC key: losing it signs everyone
out and makes stored passwords unreadable, so the next sign-in issues new ones). Back up all three
together.

The service listens on `127.0.0.1:8787`; `Caddyfile` puts it behind Caddy with automatic TLS for
`wrathplus.artivus.com.au`. `wrathplus-account.service` is an example systemd unit.

Rate limits (in memory): sign-in attempts per client address (20 per 10 minutes) and account
issues and resets per DID (10 per hour), client token redemptions per address (20 per 10
minutes). Request bodies are capped at 16 KB.

| Route | |
| --- | --- |
| `GET /` | Sign-in page, or the account page when signed in |
| `POST /oauth/login` | Start OAuth (form field `handle`) |
| `GET /oauth/callback` | OAuth return: makes the account if new, sets the session cookie |
| `POST /account/privacy` | Stop publication and remove local OAuth session (CSRF) |
| `POST /account/reset`, `POST /logout` | New game password; sign out (CSRF) |
| `GET /client/login`, `POST /client/approve`, `POST /client/token` | Client sign-in ([CLIENT-LOGIN.md](CLIENT-LOGIN.md)) |
| `GET /oauth/client-metadata.json`, `GET /oauth/jwks.json` | Client metadata and public keys |
| `POST /api/agent/login` | `{handle, app_password, reset?}` → `{account, password, realm, did, handle}` |
| `GET /healthz` | Liveness |
