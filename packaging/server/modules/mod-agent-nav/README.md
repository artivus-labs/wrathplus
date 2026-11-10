# mod-agent-nav

An AzerothCore module that answers walking-route requests from WrathPlus agent clients, using the
realm's own navigation meshes.

## Why

A WrathPlus agent walks along navmesh routes (`AgentMoveTo`, the goal runner, combat's chase).
The navmesh (`mmaps`) only comes from a multi-hour extraction of the player's own game files, and
files derived from Blizzard's data are not ours to hand out. So most agents on a public realm have
none. The realm has it already. With this module a logged-in character asks the realm for a route
on the map it stands on and gets the corners back; the client then walks them itself, as it would
a route from local mmaps.

A client with local mmaps (`WOW_NAV_MMAPS`) keeps using them and never asks.

## How it talks

No new port and no new credentials: requests are addon messages on the game connection, so only a
logged-in character can ask, and only about its own map.

The client whispers itself in the addon language,
`SendAddonMessage("WPNAV", payload, "WHISPER", UnitName("player"))`; the module swallows the
whisper and answers the same way, so replies arrive as `CHAT_MSG_ADDON` with prefix `WPNAV`. A
line (prefix, TAB, payload) is at most 255 bytes; payloads are ASCII, fields separated by single
spaces. A realm without the module delivers the whisper back unchanged, which is how a client
learns there is no route service (requests are upper-case verbs, replies lower-case, so the two
cannot be confused).

Client to realm:

| Line | Meaning |
| --- | --- |
| `H <version>` | Capability probe, sent on entering the world. Version 1. |
| `A <id> <x> <y> <r> [<x> <y> <r>]...` | Circles to route around (hostile mobs), for request `<id>` that follows. Several `A` lines add up. |
| `P <id> <flags> <fx> <fy> <fz> <tx> <ty> <tz\|->` | A route from `f` to `t`. Flags: `-`, or letters from `n` (one leg of a longer walk: the end may be moved to the nearest walkable ground) and `h` (`tz` is a guess: when no complete route ends there, the floor is looked for at other heights, 15 yards apart, ten each way, nearest first, as the client does for a target given as `x, y`). `tz` `-`: height unknown, start from `fz`. |

Realm to client:

| Line | Meaning |
| --- | --- |
| `h <version> <max distance> <max corners> <max circles> <per second> <per minute> <service radius> <max services>` | The probe's answer: the limits below. |
| `r <id> <part> <parts> <flags> <dx,dy,dz>...` | The route's corners in tenths of a yard: the first of part 1 is absolute, every later one the step from the corner before (running on across parts). Flags: `-`, or letters from `c` (complete: it reaches the end), `s` (it crosses steep ground), `t` (cut at the corner limit). |
| `e <id> <code> [detail]` | Refused: `rate`, `far`, `start`, `nomesh`, `noroute`, `bad`, `busy`. |

Coordinates are WoW yards as plain decimals (`-8907.2`); ids are 1 to 65535. Every request gets an
answer: a route or a refusal, never silence.

An exchange:

```
> H 1
< h 1 1500 256 64 5 240 2000 20
> A 7 -9380.0 100.0 23.0
> P 7 - -9460.0 64.0 56.0 -9300.0 140.0 64.0
< r 7 1 1 c -94600,640,560 <dx,dy,dz> <dx,dy,dz> ...
```

## The route

The search is the WrathPlus client's own (`crates/wrathplus-nav`), on the same Detour the core
ships: keep off steep ground unless nothing else reaches the end, swim only when needed, circles
cost ten times their length (so a route goes round when a fair detour exists and through when
none does), and a route that cannot reach the end stops at the closest point, marked incomplete. A
request marked `h` may search up to 21 heights (two searches each, gentle then steep), all on the
module's thread.
The client's tests compile this module's source and compare the two corner for corner.

## Where services are

The module also tells a character where services are on its map, as a player knows a town (and as
Playerbots knows the world). Cheating a little: it reads the world database, not what the client
has seen.

| Line | Meaning |
| --- | --- |
| `S <id> <kinds> [radius]` | Kinds, a comma list: `vendor`, `repair`, `trainer` (the asker's own class trainer), `flightmaster`, `innkeeper`, `banker`, `questgiver-hub`, `trainer_cooking`, `trainer_first_aid`, `trainer_profession` (any profession trainer). Radius in yards; none (and at most) `AgentNav.ServiceRadius`. |
| `s <id> <i> <n> <kind> <entry> <x> <y> <z> <members> <profession> <name>` | The `i`-th of `n` places, nearest first. `members`: the quest givers in a hub (1 otherwise). `profession`: the skill a profession trainer teaches, spaces as underscores (`First_Aid`), `-` otherwise. The name is the rest of the line. |
| `s <id> 0 0` | Nothing of those kinds within the radius. |

How places are chosen (`src/AgentNavServices.h`, tested against a fixture by the client's tests):

- Spawns on the asker's map and phase, within the radius, whose faction template is not hostile
  to the asker's either way (`FactionTemplateEntry::IsHostileTo`, templates only: reputation and
  at-war states are not looked at).
- The NPC flags are the spawn's own, else its template's (`ObjectMgr::ChooseCreatureFlags`):
  vendor (any vendor flag), repair, flight master, innkeeper, banker.
- `trainer`: a trainer whose default trainer (`creature_default_trainer`) is of type Class for the
  asker's class.
- Profession trainers: AzerothCore's `trainer` rows of type Tradeskill carry no requirement and
  serve weapon masters too, so the profession is read off the trainer's spells: the skill line
  they require (`trainer_spell.ReqSkillLine`, set on every rank and recipe past the first) and the
  skill lines they step (the "Apprentice X" spells' `SPELL_EFFECT_SKILL_STEP`, one learned spell
  down), the most common that is a profession or secondary skill (SkillLine.dbc categories 11 and
  9). Cooking is 185, First Aid 129.
- A quest hub: a quest giver with at least 3 quest givers (itself included) within 40 yards, given
  once, at its nearest member.
- A spawn offering several services is one place per kind. Nearest first, at most
  `AgentNav.MaxServices`.

The spawn table is read once at startup, on the world thread, from what ObjectMgr has already
loaded (no database query, then or per request), and handed to the module's thread, which answers
from it. Spawns added later (`.npc add`) are not in it until a restart. Requests count toward the
same per-character rate as routes.

The zone of each place is not sent: the realm only knows it by loading terrain, and the client
names it from its own world map (`AgentServices`' `zone`).

## Threads

The request arrives in the chat handler, on the world thread. There it is only parsed, checked
against the limits and queued. The search runs on the module's own thread, over its own copy of
the navmesh tiles, read from `<DataDir>/mmaps` as routes need them: it shares nothing with the
maps' meshes (which map threads add tiles to as grids load, and whose queries are per map and not
thread safe). Finished routes go back through a queue and are sent from the world update. So a
search cannot stall the world however long it runs; the limits bound the queue, the memory and
each search.

## Limits

They protect a shared realm; all are in `conf/mod_agent_nav.conf.dist`.

| Setting | Default | |
| --- | --- | --- |
| `AgentNav.QueriesPerSecond`, `AgentNav.QueriesPerMinute` | 5, 240 | Per character. A WrathPlus client sends at most four a second, and that only while chasing something that moves. Refused requests (`rate`) are not counted. |
| `AgentNav.MaxDistance` | 1500 | Yards, start to end in a straight line (`far`). Bounds the tiles one request loads (36). Clients walk longer ways a leg at a time. |
| `AgentNav.MaxStartDistance` | 100 | A route starts near the character asking (`start`). |
| `AgentNav.MaxWaypoints` | 256 | Corners in one answer; a longer route is cut and marked. About twenty fit one chat packet. |
| `AgentNav.MaxAvoidCircles` | 64 | Request size: circles per request, about eleven to a chat packet. A request line itself is at most 255 bytes. |
| `AgentNav.MaxQueued`, `AgentNav.MaxQueuedPerCharacter` | 64, 2 | Requests waiting for the search thread (`busy`). |
| `AgentNav.MaxLoadedTiles` | 128 | The module's own tiles in memory, least recently used dropped. About 1.5 MB a tile, 7 MB at most. |
| `AgentNav.SearchNodes` | 65535 | Detour's node budget for one search, the client's own figure. |
| `AgentNav.LegSnap` | 40 | For a leg (`n`), how far its end may move to reach walkable ground. |
| `AgentNav.SlowQueryMs` | 50 | Log a warning for a search this slow. |
| `AgentNav.ServiceRadius`, `AgentNav.MaxServices` | 2000, 20 | Where services are: the farthest place (yards) and the most places in one answer. |

`AgentNav.Enable = 0` turns the module off: whispers pass through untouched, so clients see no
route service.

## Install

Build it into the worldserver like any module (a standard module folder: `src/`, `conf/`). It
needs the realm's `mmaps` in the data folder; a map without them is answered `nomesh`. No
database changes. At startup the log says

```
>> mod-agent-nav: answering route requests from /path/to/data/mmaps (5 a second and 240 a minute per character, 1500 yd, 256 corners); 4321 service spawns
```

In the WrathPlus Manager it is a standard module: on **Server modules**, press **Add** on "Agent
navigation", then rebuild the image and restart the server. Agents the Manager starts on the same
machine read the meshes directly and do not need it.

## Tests

`src/AgentNavProtocol.h` (parsing, checks, chunking, the rate limit) and `src/AgentNavRoute.h`
(the search) use only the standard library and Detour. `cargo test -p wrathplus-nav` compiles them
and runs them against the client's side; with `WRATHPLUS_MMAPS=<server data>/mmaps` and
`-- --ignored` it also compares routes over real meshes.
