# Moving transports and instance enchant tooltips

## Repairs

AzerothCore `MotionTransport::CreateMoTrans` creates moving-transport GUIDs with
spawn counters independently of `SetEntry`. Query templates and arm type-15
transports using `OBJECT_FIELD_ENTRY`, not the GUID counter. Model selection
continues to use `GAMEOBJECT_DISPLAYID`; type-11 lifts retain their separate seed.

Wrath item objects have twelve enchantment slots. Random properties occupy slots
7–11; the previous seven-slot bag and equipment projections omitted those effect
lines. Project all twelve slots, reading the existing seven-cell countdown array
safely without inventing deadlines. Render names/effects from installed catalogs,
not from hardcoded bonuses or the template's random-property pool.

## Evidence and acceptance

On 2026-10-30 Shannon tested the installed play build and confirmed ships now
render and move correctly and item tooltips are correct. These reported symptoms
are accepted. Full passenger travel/crossing testing remains pending; Horde live
testing is deferred while Alliance play is prioritized. Do not treat this as
complete transport compatibility or Horde visual acceptance.

The logged Beaded Orb instance has property 2027/enchant 2307. Installed Wrath
catalogs resolve its effect as +1 Spell Power, not Spirit. Tests use catalog lookup
rather than hardcoding the rendered effect text.

Regression coverage includes the bag and equipped Lua tooltip seams, random-slot
boundaries 7/11, permanent/temporary and empty/unknown controls; template cache
queries independent of spawn counters; and installed route arm/tick tests for
Moonspray, six Horde zeppelins and Orgrim's Hammer, including every visited map,
off-map visibility, return visibility and map-seek controls.

## Remaining boundaries

Nonliving transport-passenger attachment, exact server timetable/phase parity,
and generic GUID-based tooltip/interaction consumers are separate follow-ups.
Installed display catalog resolution is not a mesh/GPU rendering test. No assets,
credentials or live captures are published with these changes.
