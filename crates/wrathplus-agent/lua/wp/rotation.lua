-- wp.rotation: what to cast in a fight, as a priority list. Loaded for every agent as the global
-- `Rotation`; Combat runs it while it fights. Each tick it casts the first entry that is known,
-- off cooldown, affordable, in range, not failing lately, and whose `when` holds:
--
--   Rotation.list = {
--     { spell = "Battle Shout", when = function(c) return not c.buff("Battle Shout") end },
--     { spell = "Rend", when = function(c) return not c.debuff("Rend") and c.target_hp > 30 end },
--     { spell = "Heroic Strike", when = function(c) return c.power >= 30 end },
--   }
--
-- The context `c`: power, power_max, mana (percent; 100 without mana), hp (percent), target_hp
-- (percent), distance (yards), buff(name) / debuff(name) (on the target; a name ending in "*"
-- matches a prefix), enemies(yards), attackers (mobs in combat with you), combo, in_combat,
-- fight_seconds, usable(spell) (the game's own IsUsableSpell: a proc is up, the stance fits) and
-- repeating(spell) (Auto Shot or Shoot already going). `target = "player"` casts an entry on
-- yourself.
--
-- Without a list of yours it uses a short default for your class, of the spells you know, kept
-- up to date as you learn more (Rotation.defaults()).
-- Make it your own: replace or edit Rotation.list in a module of yours (autoexec, so it comes
-- back after a restart). Rotation.stats counts casts and failures per spell for the fights since
-- the last Rotation.reset_stats(); Combat's fight reviews read them.
--
-- "Affordable" and "in range" are the game's own answers (IsUsableSpell: power, stance, procs;
-- IsSpellInRange), so a cast is rarely refused. A melee spell (no range of its own) waits for
-- `melee` yards; Combat also runs the list while it closes in, so an opener with a range band
-- (Charge) goes off on the way. When the game does refuse: out of range or facing away is tried
-- again in a second (Combat turns and closes meanwhile); anything else (a stance, a reagent) is
-- left alone longer each time (4 s, doubling to a minute). Only a refusal of the cast just made
-- counts: an auto-attack's "too far away" is not the spell's failure. Rotation.usable[spell](c)
-- adds a condition to a spell for any list.
--
-- Rotation.castable(spell) says whether a spell could go now, and Rotation.cast(spell [, on_self])
-- casts it if so, for code outside the list (Survive's emergency buttons).

local R = { list = nil, gcd = 1.0, stats = {}, melee = 5 }

local failed = {} -- spell -> time it may be tried again
local strikes = {} -- spell -> refusals in a row, for the backoff

R.usable = {
  ["Victory Rush"] = function(c) return c.buff("Victorious") end,
  ["Execute"] = function(c) return c.target_hp < 20 end,
}
local last_cast = 0
local book, defaults -- the spellbook's names (to their slots), and the class default made from it

-- The spells in the spellbook, name to slot (the highest rank's), rebuilt when they change.
local function slot(spell)
  if not book then
    book = {}
    for i = 1, 1000 do
      local name = GetSpellName(i, "spell")
      if not name then break end
      book[name] = i
    end
  end
  return book[spell]
end

local function known(spell)
  return slot(spell) ~= nil
end

local function aura(unit, name, filter)
  local prefix = name:sub(-1) == "*" and name:sub(1, -2)
  for i = 1, 40 do
    local n = UnitAura(unit, i, filter)
    if not n then return false end
    if n == name or (prefix and n:sub(1, #prefix) == prefix) then return true end
  end
  return false
end

local function pct(unit)
  local max = UnitHealthMax(unit) or 0
  return max > 0 and math.floor((UnitHealth(unit) or 0) * 100 / max) or 0
end

local function enemies(yards)
  local n = 0
  for _, u in ipairs(AgentNearby(yards)) do
    if u.kind == "unit" and not u.dead and u.in_combat and u.reaction ~= "friendly" then n = n + 1 end
  end
  return n
end

-- Mobs in combat with us (their target is us), within 40 yards.
local function attackers()
  local me, n = UnitGUID and UnitGUID("player"), 0
  for _, u in ipairs(AgentNearby(40)) do
    if me and u.kind == "unit" and not u.dead and u.in_combat and u.target == me and u.reaction ~= "friendly" then
      n = n + 1
    end
  end
  return n
end

local function mana_pct()
  if not UnitPowerType or UnitPowerType("player") ~= 0 then return 100 end
  local max = UnitPowerMax and UnitPowerMax("player", 0) or 0
  return max > 0 and math.floor((UnitPower("player", 0) or 0) * 100 / max) or 100
end

-- The fight's context, for the entries' `when`.
function R.context(target, started)
  return {
    power = UnitPower("player") or 0,
    power_max = UnitPowerMax and UnitPowerMax("player") or 0,
    hp = pct("player"),
    target_hp = UnitExists("target") and pct("target") or 0,
    distance = target and target.distance or 0,
    buff = function(name) return aura("player", name, "HELPFUL") end,
    debuff = function(name) return aura("target", name, "HARMFUL") end,
    enemies = enemies,
    attackers = attackers(),
    mana = mana_pct(),
    usable = function(spell) return IsUsableSpell ~= nil and IsUsableSpell(spell) and true or false end,
    repeating = function(spell) return IsAutoRepeatSpell ~= nil and IsAutoRepeatSpell(spell) and true or false end,
    combo = GetComboPoints and GetComboPoints("player", "target") or 0,
    in_combat = UnitAffectingCombat("player") and true or false,
    fight_seconds = started and (GetTime() - started) or 0,
  }
end

-- Known, off cooldown, affordable, in range: what the client can tell before casting.
local function ready(spell, c, on_self)
  if not known(spell) then return false end
  local name, _, _, cost, _, power_type, _, min_range, max_range = GetSpellInfo(spell)
  if not name then return false end
  -- By slot: the client's GetSpellCooldown takes (slot, book), not a name.
  local start, duration = GetSpellCooldown(slot(spell), "spell")
  if start and start > 0 and start + (duration or 0) > GetTime() then return false end
  if (failed[spell] or 0) > GetTime() then return false end
  local usable = R.usable[spell]
  if usable then
    local ok, yes = pcall(usable, c)
    if not (ok and yes) then return false end
  end
  -- Queued already (Heroic Strike waits for the next swing): casting again wastes the tick.
  if IsCurrentSpell and IsCurrentSpell(slot(spell), "spell") then return false end
  if IsUsableSpell then
    -- The game's own answer: enough power, the right stance, the proc it needs.
    if not IsUsableSpell(spell) then return false end
  else
    -- The cost as GetSpellInfo gives it, in the power's own units (Heroic Strike: 15 rage).
    local my_type = UnitPowerType and UnitPowerType("player") or power_type
    if cost and cost > 0 and power_type == my_type and c.power < cost then return false end
  end
  if not on_self then
    if max_range and max_range > 0 then
      -- A spell with a range band: the game's answer, else the band against our distance.
      local in_range = IsSpellInRange and UnitExists("target") and IsSpellInRange(spell, "target")
      if in_range == 0 then return false end
      if in_range ~= 1 and (c.distance > max_range or (min_range and min_range > 0 and c.distance < min_range)) then
        return false
      end
    elseif c.distance > R.melee then
      -- A melee spell has no band (the client answers "in range" for it from anywhere): reach.
      return false
    end
  end
  return true
end

-- Whether a spell could be cast now (on the target, or on ourselves).
function R.castable(spell, on_self)
  local target
  local guid = UnitGUID and UnitGUID("target")
  for _, u in ipairs(AgentNearby(60)) do
    if guid and u.guid == guid then target = u end
  end
  return ready(spell, R.context(target), on_self)
end

-- Whether the last cast's refusal has been counted already (the game says it twice: the cast
-- event and the red error line).
local judged, struck, passing = true, false, false

local function count(spell, key)
  local s = R.stats[spell] or { cast = 0, failed = 0 }
  s[key] = s[key] + 1
  R.stats[spell] = s
end

local function cast(spell, on_self, now)
  CastSpellByName(spell, on_self and "player" or nil)
  last_cast = now
  R.last = spell
  judged, struck, passing = false, false, false
  count(spell, "cast")
end

-- Cast a spell outside the list if it can go now. Returns whether it was cast.
function R.cast(spell, on_self)
  if not R.castable(spell, on_self) then return false end
  cast(spell, on_self, GetTime())
  return true
end

-- One tick in a fight: cast the first entry that should go now. Returns the spell cast, if any.
function R.tick(target, started)
  defaults = defaults or R.defaults()
  local list = R.list or defaults
  local now = GetTime()
  if now - last_cast < R.gcd then return nil end
  if UnitCastingInfo and UnitCastingInfo("player") then return nil end
  if UnitChannelInfo and UnitChannelInfo("player") then return nil end
  local c = R.context(target, started)
  for _, entry in ipairs(list) do
    local on_self = entry.target == "player"
    if ready(entry.spell, c, on_self) then
      local ok, yes = true, true
      if entry.when then ok, yes = pcall(entry.when, c) end
      if ok and yes then
        cast(entry.spell, on_self, now)
        return entry.spell
      end
    end
  end
end

-- The server refused a cast (a stance, a reagent, out of line of sight): leave it a few seconds,
-- and count it for the review.
local frame = CreateFrame("Frame")
frame:RegisterEvent("UNIT_SPELLCAST_FAILED")
frame:RegisterEvent("UI_ERROR_MESSAGE")
frame:RegisterEvent("UNIT_SPELLCAST_SUCCEEDED")
for _, event in ipairs({ "SPELLS_CHANGED", "LEARNED_SPELL_IN_TAB", "PLAYER_LEVEL_UP" }) do
  frame:RegisterEvent(event)
end
frame:SetScript("OnEvent", function(_, event, unit, spell)
  if event == "SPELLS_CHANGED" or event == "LEARNED_SPELL_IN_TAB" or event == "PLAYER_LEVEL_UP" then
    book, defaults = nil, nil
    return
  end
  if event == "UNIT_SPELLCAST_SUCCEEDED" then
    if unit == "player" and spell then strikes[spell] = nil end
    return
  end
  -- Only the cast just made: an auto-attack's "too far away" a second later is not the spell's
  -- refusal. The game says it twice (the cast event and the red error line); it counts once.
  local name = R.last
  if not name or GetTime() - last_cast > 0.6 then return end
  if not judged then
    if event == "UNIT_SPELLCAST_FAILED" and (unit ~= "player" or (spell and spell ~= name)) then return end
    judged = true
    count(name, "failed")
  end
  local message = event == "UI_ERROR_MESSAGE" and tostring(unit or ""):lower() or nil
  if message then
    -- The global cooldown or a swing in the way: not a refusal of the spell.
    if message:find("not ready") or message:find("another action") then passing = true end
    -- Out of reach, turned away, short of power: right again in a moment.
    for _, word in ipairs({ "too far", "range", "facing", "in front", "not enough", "moving" }) do
      if message:find(word, 1, true) then passing = true end
    end
  end
  if passing then
    -- Whatever the cast event already backed off is taken back.
    if struck then strikes[name] = (strikes[name] or 1) > 1 and strikes[name] - 1 or nil end
    struck = false
    failed[name] = GetTime() + 1
  elseif not struck then
    -- Anything else (a stance, a reagent, a wrong target): left alone longer each time.
    struck = true
    strikes[name] = (strikes[name] or 0) + 1
    failed[name] = GetTime() + math.min(4 * 2 ^ (strikes[name] - 1), 60)
  end
end)

function R.reset_stats()
  R.stats = {}
end

local DEFAULTS = {
  -- A warrior to level 20: Charge in (its range band keeps it to the approach), Bloodrage for the
  -- first rage, the shout, Thunder Clap when a second mob is on us, Rend on a mob that will live
  -- to bleed, and Heroic Strike with what is left over (it replaces the next swing, so it is
  -- queued once, never while one waits).
  WARRIOR = {
    { spell = "Charge", when = function(c) return not c.in_combat and c.distance >= 10 and c.distance <= 22 end },
    { spell = "Bloodrage", target = "player", when = function(c) return c.in_combat and c.power < 20 and c.hp > 60 end },
    { spell = "Victory Rush" },
    { spell = "Execute", when = function(c) return c.target_hp < 20 end },
    { spell = "Overpower", when = function(c) return c.usable("Overpower") end },
    { spell = "Battle Shout", target = "player", when = function(c) return not c.buff("Battle Shout") end },
    { spell = "Thunder Clap", when = function(c) return c.attackers >= 2 and not c.debuff("Thunder Clap") end },
    { spell = "Rend", when = function(c) return not c.debuff("Rend") and c.target_hp > 40 end },
    { spell = "Heroic Strike", when = function(c) return c.power >= 25 end },
  },
  PALADIN = {
    { spell = "Holy Light", target = "player", when = function(c) return c.hp < 50 end },
    { spell = "Devotion Aura", target = "player", when = function(c) return not c.buff("Devotion Aura") end },
    { spell = "Blessing of Might", target = "player", when = function(c) return not c.buff("Blessing of Might") end },
    { spell = "Seal of Righteousness", target = "player", when = function(c) return not c.buff("Seal of*") end },
    { spell = "Judgement of Light", when = function(c) return c.buff("Seal of*") end },
    { spell = "Crusader Strike" },
  },
  HUNTER = {
    { spell = "Hunter's Mark", when = function(c) return not c.debuff("Hunter's Mark") end },
    { spell = "Serpent Sting", when = function(c) return not c.debuff("Serpent Sting") and c.target_hp > 30 end },
    { spell = "Arcane Shot" },
    { spell = "Raptor Strike", when = function(c) return c.distance < 5 end },
    { spell = "Auto Shot", when = function(c) return c.distance >= 8 and not c.repeating("Auto Shot") end },
  },
  ROGUE = {
    { spell = "Slice and Dice", target = "player", when = function(c) return c.combo >= 2 and not c.buff("Slice and Dice") end },
    { spell = "Eviscerate", when = function(c) return c.combo >= 4 or (c.combo >= 2 and c.target_hp < 25) end },
    { spell = "Sinister Strike" },
  },
  PRIEST = {
    { spell = "Lesser Heal", target = "player", when = function(c) return c.hp < 50 end },
    { spell = "Power Word: Fortitude", target = "player", when = function(c) return not c.buff("Power Word: Fortitude") end },
    { spell = "Power Word: Shield", target = "player", when = function(c) return not c.buff("Power Word: Shield") and not c.debuff("Weakened Soul") end },
    { spell = "Renew", target = "player", when = function(c) return c.hp < 70 and not c.buff("Renew") end },
    { spell = "Shadow Word: Pain", when = function(c) return not c.debuff("Shadow Word: Pain") and c.target_hp > 30 end },
    { spell = "Mind Blast" },
    { spell = "Smite" },
    -- Low on mana: the wand, once (it repeats by itself).
    { spell = "Shoot", when = function(c) return c.mana < 25 and not c.repeating("Shoot") end },
  },
  MAGE = {
    { spell = "Arcane Intellect", target = "player", when = function(c) return not c.buff("Arcane Intellect") end },
    { spell = "Frost Armor", target = "player", when = function(c) return not c.buff("Frost Armor") end },
    { spell = "Frost Nova", when = function(c) return c.distance < 8 end },
    { spell = "Fire Blast", when = function(c) return c.target_hp < 30 end },
    { spell = "Frostbolt" },
    { spell = "Fireball" },
    -- Out of mana: the wand, once (it repeats by itself).
    { spell = "Shoot", when = function(c) return c.mana < 25 and not c.repeating("Shoot") end },
  },
  WARLOCK = {
    { spell = "Demon Skin", target = "player", when = function(c) return not c.buff("Demon Skin") end },
    { spell = "Corruption", when = function(c) return not c.debuff("Corruption") and c.target_hp > 30 end },
    { spell = "Curse of Agony", when = function(c) return not c.debuff("Curse of Agony") and c.target_hp > 30 end },
    { spell = "Immolate", when = function(c) return not c.debuff("Immolate") end },
    { spell = "Drain Life", when = function(c) return c.hp < 50 end },
    { spell = "Shadow Bolt" },
    -- Out of mana: the wand, once (it repeats by itself).
    { spell = "Shoot", when = function(c) return c.mana < 25 and not c.repeating("Shoot") end },
  },
  SHAMAN = {
    { spell = "Healing Wave", target = "player", when = function(c) return c.hp < 50 end },
    { spell = "Lightning Shield", target = "player", when = function(c) return not c.buff("Lightning Shield") end },
    { spell = "Earth Shock", when = function(c) return c.distance < 20 end },
    { spell = "Lightning Bolt", when = function(c) return not c.in_combat or c.distance > 8 end },
  },
  DRUID = {
    { spell = "Healing Touch", target = "player", when = function(c) return c.hp < 50 end },
    { spell = "Rejuvenation", target = "player", when = function(c) return c.hp < 70 and not c.buff("Rejuvenation") end },
    { spell = "Mark of the Wild", target = "player", when = function(c) return not c.buff("Mark of the Wild") end },
    { spell = "Thorns", target = "player", when = function(c) return not c.buff("Thorns") end },
    { spell = "Moonfire", when = function(c) return not c.debuff("Moonfire") end },
    { spell = "Wrath" },
  },
  DEATHKNIGHT = {
    { spell = "Icy Touch", when = function(c) return not c.debuff("Frost Fever") end },
    { spell = "Plague Strike", when = function(c) return not c.debuff("Blood Plague") end },
    { spell = "Death Strike", when = function(c) return c.hp < 60 end },
    { spell = "Blood Strike" },
    { spell = "Death Coil", when = function(c) return c.power >= 60 end },
  },
}

-- How far from the target each class fights (yards): casters and hunters from range.
R.RANGE = { HUNTER = 25, PRIEST = 25, MAGE = 25, WARLOCK = 25, DRUID = 25, SHAMAN = 20 }

-- The class's default list, of the spells this character knows.
function R.defaults()
  local _, class = UnitClass("player")
  local out = {}
  for _, entry in ipairs(DEFAULTS[class or ""] or {}) do
    if known(entry.spell) then out[#out + 1] = entry end
  end
  return out
end

-- The spells in the list, for a person or an LLM reading it.
function R.describe()
  local names = {}
  for _, e in ipairs(R.list or defaults or R.defaults()) do names[#names + 1] = e.spell end
  return table.concat(names, " > ")
end

return R
