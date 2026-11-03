# mod-quest-minimap-points

An AzerothCore module that brings back the quest marker on the 3.3.5a minimap, and the arrow at
the minimap's edge that points to it.

## Why it is missing

A 3.3.5a client draws one minimap marker per quest, and takes it only from the quest's point:
`quest_template.POIContinent`, `POIx` and `POIy`, sent in the quest query response. The shaded
objective areas on the world map come from `quest_poi`, and the client never draws those on the
minimap. Retail servers sent a point for most quests. AzerothCore's world database has one for
105 of 9,464 quests, so a stock client on an AzerothCore realm almost never shows the marker.

## What it does

One world database update fills the point for every quest that has none, from the quest's own
`quest_poi` areas:

- the centre of its largest objective area (most polygon vertices, then the lowest area id);
- or, with no objective area, the centre of its hand-in area.

Checked against the 104 quests that still carry Blizzard's point, this lands within 50 yards of
it for 81 and within 150 for 92. On the pinned world database it gives 8,861 more quests a point
(8,966 of 9,464). Quests whose point is not (0, 0) are left alone, so curated points survive.

At startup the worldserver log says how many quests have a point:

```
>> mod-quest-minimap-points: 8966 of 9464 quests have a minimap point
```

No client change is involved: any 3.3.5a client gains the marker.

## Install

Build it into the worldserver like any module. The update applies on the next start
(`Updates.EnableDatabases` including the world database, the default), then restart so the
quest templates are reloaded. Clients pick the points up when they next query a quest; a stock
client keeps quest replies in `Cache/WDB`, so clear `questcache.wdb` once.

In the WrathPlus Manager: on **Server modules**, under Custom modules, enter this folder's full
path and press **Add source**, then rebuild the image and restart the server.

## Removing it

The update is not undone by disabling the module. To clear the filled points, restore
`quest_template` from the core's world database, or zero the points that match no Blizzard
point you keep.
