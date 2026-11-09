# WrathPlus account service

Sign in with ATProto, get a game account on the beta realm (`wrathplus.artivus.com.au`).

- **People** sign in with ATProto OAuth on the web page: a confidential client
  (`@atproto/oauth-client-node`, `private_key_jwt` with ES256, PAR, PKCE and DPoP). The client
  metadata is published at `https://wrathplus.artivus.com.au/oauth/client-metadata.json`.
- **Agents** sign in with an app password: `POST /api/agent/login`.

Either way the identity is proven first; the DID is the permanent key. The service then makes (or
finds) the DID's one game account on the realm over SOAP, sets a new random password and shows it
**once**. Signing in again issues a new password; the old one stops working.

```sh
curl -s https://wrathplus.artivus.com.au/api/agent/login \
  -H 'content-type: application/json' \
  -d '{"handle":"mybot.bsky.social","app_password":"xxxx-xxxx-xxxx-xxxx"}'
# {"account":"MYBOT","password":"K7Q2…","realm":"wrathplus.artivus.com.au:3724","did":"did:plc:…","handle":"mybot.bsky.social"}
```

Then point a 3.3.5a client (or a WrathPlus agent) at `set realmlist wrathplus.artivus.com.au`
and log in with `account` / `password`. Passwords are not case-sensitive in game.

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

## Records

Lexicons (in `lexicons/`, authority `wrathplus.artivus.com.au`):

- `au.com.artivus.wrathplus.character`: one record per character, key `<realm>:<guid>`.
- `au.com.artivus.wrathplus.milestone`: a level reached, an achievement earned, a quest turned in
  (quests only with `PUBLISH_QUESTS=1`), key `<realm>:<guid>:<kind>:<value>` (quests add the time).

The OAuth scope is `atproto` plus `repo:<collection>?action=create&action=update` for these two
collections only. The service polls the realm module (`mod-atproto-identity`, `.atproto events`
over SOAP), writes the records with the person's stored OAuth session, then acknowledges them.
Record keys come from the event's content, so a repeat delivery overwrites instead of duplicating.

**Agents** have no stored write credential (their session is deleted at sign-in), so the service
publishes nothing for them in v1; their events are acknowledged and dropped. An agent that wants
records writes them itself with its own app-password session (`com.atproto.repo.putRecord`, same
collections and keys).

Publishing the lexicons (the `_lexicon.wrathplus.artivus.com.au` TXT record and
`com.atproto.lexicon.schema` records) is a separate step that needs the DNS owner.

## Run

Needs Bun 1.4 or newer (1.2 cannot load the OAuth library's undici dependency).

```sh
bun install --frozen-lockfile
bun test
bun run typecheck
SOAP_CREDENTIALS_FILE=/etc/wrathplus/soap-account bun run src/index.ts
```

Configuration is environment only (`account.env.example`); the SOAP account comes from a file
holding `USER:PASSWORD`, which must be mode 0600 (the service refuses a readable one). State is
one SQLite file and the OAuth signing keys (`oauth-keys.json`, generated 0600 on first start) in
`ACCOUNT_DATA_DIR`: back both up; losing the keys signs everyone out of record publishing.

The service listens on `127.0.0.1:8787`; `Caddyfile` puts it behind Caddy with automatic TLS for
`wrathplus.artivus.com.au`. `wrathplus-account.service` is an example systemd unit.

Rate limits (in memory): sign-in attempts per client address (20 per 10 minutes) and account
issues per DID (10 per hour). Request bodies are capped at 16 KB.

| Route | |
| --- | --- |
| `GET /` | Sign-in page |
| `POST /oauth/login` | Start OAuth (form field `handle`) |
| `GET /oauth/callback` | OAuth return; shows the account and password once |
| `GET /oauth/client-metadata.json`, `GET /oauth/jwks.json` | Client metadata and public keys |
| `POST /api/agent/login` | `{handle, app_password}` → `{account, password, realm, did, handle}` |
| `GET /healthz` | Liveness |
