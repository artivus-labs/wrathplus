# mod-party-quest-share

An AzerothCore module for realms with [mod-playerbots](https://github.com/mod-playerbots/mod-playerbots):
when someone in a party shares a quest, every playerbot in the party that can take it accepts.

## Why bots ignore a shared quest

When a player shares a quest, the core checks each party member and sends those who can take it
the quest's details, to accept or decline. A playerbot has no client to answer. mod-playerbots
answers for a bot only when the share comes from the bot's **master**:

- A bot you added yourself (`.playerbots bot addclass`, an alt) has you as its master, so your
  shares work.
- A random bot you invite takes the **party leader** as its master. A share from anyone else in
  the party gets no answer: the bot neither accepts nor declines, and the core then reports it
  as busy for every later share.

Where mod-playerbots does answer, the bot may also whisper "I have this quest" after it has
accepted.

## What it does

The module answers for the bot at the moment the core offers it the quest. It runs the core's own
accept handler on the bot's session, as a client pressing Accept would, so every rule is the
core's:

- The core offers the quest only to a member on the sharer's map who is not on the quest and has
  not finished it, can take it (level, class, race, skill, reputation, earlier quests), has room
  in the quest log and is not answering another share.
- The accept handler checks again that the sharer can share the quest and the bot can take it,
  and that both are alive; it refuses a quest whose starting item can be sold.
- The sharer sees the usual "... has accepted your quest". If the accept handler refuses, the
  module declines for the bot ("... has declined your quest"), so the bot is free for the next
  share.

It works whoever shares: the leader, another player, or another bot in the party. Because the bot
has answered before mod-playerbots looks at the share, mod-playerbots finds nothing to answer and
stays quiet, so the "I have this quest" whisper goes too (and so does its "Quest accepted").

Real players are not affected: the module acts only on sessions without a client, and only on a
quest offer that is a share from a member of the bot's own party. Bots outside the party are
never offered the quest by the core.

Without mod-playerbots the module builds and does nothing.

## How it hooks in

`ServerScript::OnPacketSent`, on `SMSG_QUESTGIVER_QUEST_DETAILS` queued for a client-less
session (`WorldSession::IsHeadless`). The core has no quest-share hook, and a hook on the
sharer's `CMSG_PUSHQUESTTOPARTY` runs before the core's handler: accepting there would copy the
core's checks and make the core report every bot as already on the quest. The details packet is
the core's own "this member may take it" decision.

The cost is one opcode comparison per packet sent. The accept runs inside the sharer's share
handler, on the world thread, once per eligible bot per share. No timers, no database access.

## Config

`conf/mod_party_quest_share.conf.dist`:

| Setting | Default | What |
| --- | --- | --- |
| `PartyQuestShare.Enable` | 1 | 0 leaves shared quests to mod-playerbots |

At startup the worldserver log says

```
>> mod-party-quest-share: party bots accept shared quests
```

## Install

Build it into the worldserver like any module. It has no SQL.

In the WrathPlus Manager it is a standard module: on **Server modules**, press **Add** on
"Party bots take shared quests", then rebuild the image and restart the server.

## Removing it

Unload the module. It stores nothing.
