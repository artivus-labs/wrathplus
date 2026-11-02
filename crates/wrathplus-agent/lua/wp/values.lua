-- wp.values: client-visible world state for agent logic, read through the stock 3.3.5 API only.
-- Where Playerbots reads the server (threat tables, every nearby object, spawn data), these read
-- what a player's client knows: its own units, party and raid, auras, cooldowns and bags.
--
--   local V = require("wp.values")
--   V.health("player")            --> 0.83
--   V.lowest_party_member(0.6)    --> "party2", 0.41
--   V.spell_ready("Heroic Strike", "target")

local V = {}

-- Health as a fraction (nil if the unit doesn't exist).
function V.health(unit)
  local max = UnitHealthMax(unit)
  if not UnitExists(unit) or not max or max == 0 then return nil end
  return UnitHealth(unit) / max
end

-- Power (mana, rage, energy, runic power) as a fraction, and its type token.
function V.power(unit)
  local max = UnitPowerMax(unit)
  if not UnitExists(unit) or not max or max == 0 then return nil end
  local _, token = UnitPowerType(unit)
  return UnitPower(unit) / max, token
end

-- The group's units: "player" plus party1-4, or raid1-40 in a raid.
function V.group()
  local units = { "player" }
  if GetNumRaidMembers() > 0 then
    units = {}
    for i = 1, GetNumRaidMembers() do units[#units + 1] = "raid" .. i end
  else
    for i = 1, GetNumPartyMembers() do units[#units + 1] = "party" .. i end
  end
  return units
end

-- The living group member with the lowest health fraction below `threshold` (default 1).
function V.lowest_party_member(threshold)
  local best, best_hp
  for _, unit in ipairs(V.group()) do
    local hp = V.health(unit)
    if hp and not UnitIsDeadOrGhost(unit) and UnitIsConnected(unit) and hp < (threshold or 1)
        and (not best_hp or hp < best_hp) then
      best, best_hp = unit, hp
    end
  end
  return best, best_hp
end

-- The first aura on `unit` named `name`: its count, expiry and caster. filter: "HELPFUL"/"HARMFUL".
function V.aura(unit, name, filter)
  local query = (filter == "HARMFUL") and UnitDebuff or UnitBuff
  for i = 1, 40 do
    local aura, _, _, count, _, duration, expires, caster = query(unit, i)
    if not aura then return nil end
    if aura == name then
      return { count = count, duration = duration, expires = expires, caster = caster }
    end
  end
end

-- Seconds until a spell is off cooldown (0 when ready).
function V.cooldown(spell)
  local start, duration, enabled = GetSpellCooldown(spell)
  if not start or enabled == 0 then return math.huge end
  if start == 0 then return 0 end
  return math.max(0, start + duration - GetTime())
end

-- Ready to cast on `unit`: known, usable (power, form), off cooldown and in range.
function V.spell_ready(spell, unit)
  local usable, no_power = IsUsableSpell(spell)
  if not usable or no_power then return false end
  if V.cooldown(spell) > 0 then return false end
  if unit and UnitExists(unit) then
    local in_range = IsSpellInRange(spell, unit)
    if in_range == 0 then return false end
  end
  return true
end

-- Roughly how near a unit is, from the client's interaction ranges: "near" (duel or trade range,
-- about 10-11 yd), "follow" (about 28 yd) or "far".
function V.range(unit)
  if not UnitExists(unit) then return nil end
  if CheckInteractDistance(unit, 3) then return "near" end   -- duel, ~10 yd
  if CheckInteractDistance(unit, 2) then return "near" end   -- trade, ~11 yd
  if CheckInteractDistance(unit, 4) then return "follow" end -- follow, ~28 yd
  return "far"
end

-- Is the target something to fight: exists, hostile or attackable, alive.
function V.attackable(unit)
  unit = unit or "target"
  return UnitExists(unit) and UnitCanAttack("player", unit) and not UnitIsDeadOrGhost(unit)
end

-- My threat on `unit` (default target): status 0-3 and percent, from the client's own reading.
function V.threat(unit)
  local tanking, status, percent = UnitDetailedThreatSituation("player", unit or "target")
  return status or 0, percent or 0, tanking and true or false
end

-- Free bag slots across the backpack and bags.
function V.free_bag_slots()
  local free = 0
  for bag = 0, 4 do
    local slots = GetContainerNumFreeSlots(bag)
    free = free + (slots or 0)
  end
  return free
end

-- Nearby objects the client knows (AgentNearby: units, players, game objects, corpses), nearest
-- first, filtered by opts: kind ("unit", "player", "gameobject", "corpse"), reaction ("hostile",
-- "neutral", "friendly"), alive (true), max (yards), quest ("available", "incomplete", "reward"),
-- npc ("vendor", "trainer"...), name (exact), lootable (true).
function V.nearby(opts)
  opts = opts or {}
  local out = {}
  for _, o in ipairs(AgentNearby(opts.max)) do
    local keep = (not opts.kind or o.kind == opts.kind)
      and (not opts.reaction or o.reaction == opts.reaction)
      and (not opts.alive or not o.dead)
      and (not opts.quest or o.quest == opts.quest)
      and (not opts.name or o.name == opts.name)
      and (not opts.lootable or o.lootable)
    if keep and opts.npc then
      keep = false
      for _, service in ipairs(o.npc or {}) do if service == opts.npc then keep = true end end
    end
    if keep then out[#out + 1] = o end
  end
  return out
end

-- The nearest object matching opts (see V.nearby), or nil.
function V.nearest(opts) return V.nearby(opts)[1] end

-- Where the player is: zone, subzone and map position (0-1).
function V.position()
  local x, y = GetPlayerMapPosition("player")
  return { zone = GetRealZoneText(), subzone = GetSubZoneText(), x = x, y = y,
    facing = GetPlayerFacing and GetPlayerFacing() or nil }
end

return V
