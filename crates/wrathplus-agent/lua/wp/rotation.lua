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
-- The context `c`: power, power_max, hp (percent), target_hp (percent), distance (yards),
-- buff(name) / debuff(name) (on the target; a name ending in "*" matches a prefix), enemies(yards),
-- combo, in_combat, fight_seconds. `target = "player"` casts an entry on yourself.
--
-- Without a list of yours it uses a short default for your class, of the spells you know, kept
-- up to date as you learn more (Rotation.defaults()).
-- Make it your own: replace or edit Rotation.list in a module of yours (autoexec, so it comes
-- back after a restart). Rotation.stats counts casts and failures per spell for the fights since
-- the last Rotation.reset_stats(); Combat's fight reviews read them.

local R = { list = nil, gcd = 1.0, stats = {} }

local failed = {} -- spell -> time it may be tried again
local last_cast = 0
local book, defaults -- the spellbook's names, and the class default made from it

-- The spells in the spellbook, rebuilt when they change (a level, a trainer).
local function known(spell)
  if not book then
    book = {}
    for i = 1, 1000 do
      local name = GetSpellName(i, "spell")
      if not name then break end
      book[name] = true
    end
  end
  return book[spell] == true
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
  local start, duration = GetSpellCooldown(spell)
  if start and start > 0 and start + (duration or 0) > GetTime() then return false end
  if (failed[spell] or 0) > GetTime() then return false end
  local my_type = UnitPowerType and UnitPowerType("player") or power_type
  if cost and cost > 0 and power_type == my_type and c.power < cost then return false end
  if not on_self and max_range and max_range > 0 then
    if c.distance > max_range + 2 or (min_range and min_range > 0 and c.distance < min_range) then return false end
  end
  return true
end

local function count(spell, key)
  local s = R.stats[spell] or { cast = 0, failed = 0 }
  s[key] = s[key] + 1
  R.stats[spell] = s
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
        CastSpellByName(entry.spell, on_self and "player" or nil)
        last_cast = now
        R.last = entry.spell
        count(entry.spell, "cast")
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
for _, event in ipairs({ "SPELLS_CHANGED", "LEARNED_SPELL_IN_TAB", "PLAYER_LEVEL_UP" }) do
  frame:RegisterEvent(event)
end
frame:SetScript("OnEvent", function(_, event, unit, spell)
  if event == "SPELLS_CHANGED" or event == "LEARNED_SPELL_IN_TAB" or event == "PLAYER_LEVEL_UP" then
    book, defaults = nil, nil
    return
  end
  local name = event == "UNIT_SPELLCAST_FAILED" and unit == "player" and spell or (event == "UI_ERROR_MESSAGE" and R.last)
  if name and GetTime() - last_cast < 1.5 then
    failed[name] = GetTime() + 4
    count(name, "failed")
  end
end)

function R.reset_stats()
  R.stats = {}
end

local DEFAULTS = {
  WARRIOR = {
    { spell = "Charge", when = function(c) return not c.in_combat end },
    { spell = "Victory Rush" },
    { spell = "Execute", when = function(c) return c.target_hp < 20 end },
    { spell = "Battle Shout", target = "player", when = function(c) return not c.buff("Battle Shout") end },
    { spell = "Rend", when = function(c) return not c.debuff("Rend") and c.target_hp > 30 end },
    { spell = "Thunder Clap", when = function(c) return c.enemies(8) >= 2 end },
    { spell = "Heroic Strike", when = function(c) return c.power >= 30 end },
  },
  PALADIN = {
    { spell = "Holy Light", target = "player", when = function(c) return c.hp < 40 end },
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
    { spell = "Auto Shot", when = function(c) return c.distance >= 8 end },
  },
  ROGUE = {
    { spell = "Slice and Dice", target = "player", when = function(c) return c.combo >= 2 and not c.buff("Slice and Dice") end },
    { spell = "Eviscerate", when = function(c) return c.combo >= 4 or (c.combo >= 2 and c.target_hp < 25) end },
    { spell = "Sinister Strike" },
  },
  PRIEST = {
    { spell = "Lesser Heal", target = "player", when = function(c) return c.hp < 40 end },
    { spell = "Power Word: Fortitude", target = "player", when = function(c) return not c.buff("Power Word: Fortitude") end },
    { spell = "Power Word: Shield", target = "player", when = function(c) return not c.buff("Power Word: Shield") and not c.debuff("Weakened Soul") end },
    { spell = "Renew", target = "player", when = function(c) return c.hp < 70 and not c.buff("Renew") end },
    { spell = "Shadow Word: Pain", when = function(c) return not c.debuff("Shadow Word: Pain") and c.target_hp > 30 end },
    { spell = "Mind Blast" },
    { spell = "Smite" },
  },
  MAGE = {
    { spell = "Arcane Intellect", target = "player", when = function(c) return not c.buff("Arcane Intellect") end },
    { spell = "Frost Armor", target = "player", when = function(c) return not c.buff("Frost Armor") end },
    { spell = "Frost Nova", when = function(c) return c.distance < 8 end },
    { spell = "Fire Blast", when = function(c) return c.target_hp < 30 end },
    { spell = "Frostbolt" },
    { spell = "Fireball" },
  },
  WARLOCK = {
    { spell = "Demon Skin", target = "player", when = function(c) return not c.buff("Demon Skin") end },
    { spell = "Corruption", when = function(c) return not c.debuff("Corruption") and c.target_hp > 30 end },
    { spell = "Curse of Agony", when = function(c) return not c.debuff("Curse of Agony") and c.target_hp > 30 end },
    { spell = "Immolate", when = function(c) return not c.debuff("Immolate") end },
    { spell = "Shadow Bolt" },
  },
  SHAMAN = {
    { spell = "Healing Wave", target = "player", when = function(c) return c.hp < 40 end },
    { spell = "Lightning Shield", target = "player", when = function(c) return not c.buff("Lightning Shield") end },
    { spell = "Earth Shock", when = function(c) return c.distance < 20 end },
    { spell = "Lightning Bolt", when = function(c) return not c.in_combat or c.distance > 8 end },
  },
  DRUID = {
    { spell = "Healing Touch", target = "player", when = function(c) return c.hp < 40 end },
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
