# mod-atproto-identity

An AzerothCore module for the WrathPlus open beta: game accounts that belong to ATProto identities
(Bluesky and other PDS accounts). The [account service](../../../../services/account) proves who
someone is and makes their account; this module is the realm's side of it.

- **Who owns an account.** `account_atproto` (auth database) maps an account id to a DID (the
  permanent identity) and its handle. Only the console command `.atproto link` writes it.
- **In game.** A linked player entering the world reads `Signed in as @alice.bsky.social`.
  `.atproto whoami` repeats it, with the DID.
- **Progress.** Level-ups, quest turn-ins and achievements of characters on linked accounts are
  queued for the account service, which publishes them as ATProto records in the player's own
  repository. Bots and unlinked accounts produce nothing.

## Commands

| Command | Who | What |
| --- | --- | --- |
| `.atproto link ACCOUNT did handle` | console / SOAP | Link (or relink with a new handle); one account per DID |
| `.atproto unlink ACCOUNT` | console / SOAP | Remove the link |
| `.atproto events [1-500]` | console / SOAP | The oldest queued events, one line each |
| `.atproto ack ID` | console / SOAP | Forget events up to and including ID |
| `.atproto whoami` | players | Which identity this account belongs to |

The console commands are `SEC_CONSOLE`: no in-game account can run them; SOAP runs as console.
Every argument is checked again here (account `[A-Z0-9]{1,16}`, `did:plc:` + 24 base32 or
`did:web:` + host name, handles as ATProto handles), so nothing the service passes on can reach
SQL unvalidated.

An event line is

```
EV <id> <unix time> <did> <character guid> <login|level|quest|achievement> <value> <level> <race> <class> <gender> <name> [<title>]
```

where value is the new level, the quest id or the achievement id, and title (the rest of the line)
is the quest's or achievement's name.

## Design

- **SOAP, not a database connection, for the service.** The realm's MySQL stays inside the Docker
  network; the service needs only the SOAP account it already uses to make game accounts. The
  module owns its tables and validates every write.
- **Nothing waits on the database during play.** Links and unacknowledged events are loaded once
  at startup and then kept in memory (one mutex: player hooks run on map threads). Event rows are
  written with `CharacterDatabase.Execute`, which is asynchronous; `.atproto ack` deletes them the
  same way. `.atproto link` resolves the account name with `AccountMgr::GetId`, a synchronous
  lookup, like the core's own `.account` commands; it runs once per sign-in.
- **Polling, not pushing.** The worldserver makes no outbound HTTP calls: the service polls
  `.atproto events` every few seconds and acknowledges what it has published. If it is down, events
  wait (up to `AtprotoIdentity.Events.MaxQueued`, then new ones are dropped and logged), and survive
  a worldserver restart in `atproto_event`.
- **At least once.** An event can be handed out again (a crash between publish and ack). The
  service derives each record's key from the event's content, so a repeat overwrites rather than
  duplicates. Event ids restart from the highest queued id after a restart; they order events
  but are not identities.

## Install

Build it into the worldserver like any module (it is a standard module folder: `src/`, `conf/`,
`data/sql/`). The auth and characters updates apply on the next start with
`Updates.EnableDatabases` covering both (the default). Config: `conf/mod_atproto_identity.conf.dist`.

SOAP must be enabled and bound to loopback or the Docker network only, with a gmlevel-3 account
reserved for the account service.

## Removing it

Unload the module; the tables stay. Drop `account_atproto` (auth) and `atproto_event`
(characters) to remove the links and any queued events. Game accounts are not affected.
