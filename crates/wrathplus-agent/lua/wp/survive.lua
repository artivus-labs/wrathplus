-- wp.survive: staying alive, as a player does without thinking about it. Loaded for every agent
-- as the global `Survive`; Combat, Goals and Revive ask it, and so can your own modules. Everything
-- it reads is the client's own: what is in sight, our health over the last seconds, the bags.
--
-- Before a pull:
--   Survive.pull_ok(u) → ok, why   whether fighting unit `u` (an AgentNearby entry) is a fight we
--       should start: it suits us (Survive.fits: alive, not friendly, not another's tap, no elite,
--       rare or critter, inside our level band), we are ready (Survive.ready: health, more of it
--       for a mob above us, mana, a main hand that is not broken), what stands close enough to
--       join does not make it too much at once (Survive.crowded: a mob of our level weighs 1,
--       less below us, more above, and a pull may weigh `max_pull`: one mob of our level with a
--       low one beside it, not two of our level), and it is not where we died a few minutes
--       ago. A unit already attacking us is always fair.
--   Survive.band() → lowest, highest   the levels we pull: above the grey level, up to `above`
--       over ours; every recent death takes `death_penalty` levels off the top for
--       `death_window` seconds, and every close call (a fight fled, or won below `scare_below`
--       health) one level, to two below ours at most: mobs that nearly kill us are too much for
--       this character as it is geared, whatever its level says.
--   Survive.deaths, Survive.recent_deaths(), Survive.scares   where and to what we died lately,
--       and the close calls.
-- In a fight (Combat calls Survive.fight_tick every tick):
--   it watches our health and the attackers' (everything in combat with us, the fight's target or
--   not), and from the rates says whether we are losing (Survive.assess). Losing or low, it uses a
--   healthstone or healing potion from the bags, then the class's emergency buttons
--   (Survive.EMERGENCY, of the spells known), and when the fight is lost and the run can be
--   survived it tells Combat to flee: snare the mob if the class can (Survive.SNARE), then run to
--   Survive.escape_point(), away from the attackers and clear of other hostiles. A run that
--   cannot be survived is not started: the fight goes on with everything it has.
-- Between fights:
--   Survive.eat() uses food (and drink, for mana) from the bags, else a bandage, else sits.
--
-- The thresholds are fields; set them live. `Survive.on = false` turns the judgement off (every
-- pull is fair, no potions, no fleeing). `Survive.log` keeps its last few decisions.

local S = {
  on = true,
  -- Pulls: how far above our level, how much a pull may weigh with what would join it (a mob of
  -- our level is 1), and the health and mana (fractions) a fight is started at.
  above = 1, max_pull = 1.3, pull_hp = 0.7, pull_mana = 0.4,
  -- A hostile this close to the target (yards) joins its fight whatever its aggro radius.
  assist = 10,
  -- Deaths: how long one counts as recent, how far round its spot nothing is pulled, and the
  -- levels each takes off the band.
  death_window = 600, death_radius = 50, death_penalty = 2,
  -- A fight won below this health is a close call (as a fight fled is).
  scare_below = 0.3,
  -- In a fight: use a potion below this health when losing (or below `critical` anyway); below
  -- `flee_below` a losing fight is left. `window` is the seconds the rates are measured over.
  potion_below = 0.4, critical = 0.25, flee_below = 0.5, window = 6,
  -- A fight is lost for sure when they would outlast us by this factor.
  decisive = 1.5,
  -- The run: how long a mob at our heels keeps hitting before it gives up (it runs as fast as
  -- we do, and follows until far from its home), unsnared and snared, and how far each leg goes.
  run_seconds = 14, snared_seconds = 4, run_yards = 45,
  -- What we deal per second, learned from the fights won (nil until one is).
  dps = nil,
  deaths = {}, scares = {}, log = {},
}

local function now() return GetTime() end

local function note(text)
  S.log[#S.log + 1] = string.format("%.1f %s", now(), text)
  if #S.log > 12 then table.remove(S.log, 1) end
end

local function far(a, b) return math.sqrt((a.x - b.x) ^ 2 + (a.y - b.y) ^ 2) end
local function level() return UnitLevel("player") or 1 end
local function seen(yards) return AgentNearby and AgentNearby(yards) or {} end

local function fraction(value, max)
  if not max or max <= 0 then return 1 end
  return (value or 0) / max
end

function S.hp() return fraction(UnitHealth("player"), UnitHealthMax("player")) end

-- Mana as a fraction; 1 for a class without it.
function S.mana()
  if UnitPowerType and UnitPowerType("player") ~= 0 then return 1 end
  return fraction(UnitPower("player", 0), UnitPowerMax and UnitPowerMax("player", 0))
end

-- 3.3.5's grey level: mobs at or below it give no XP.
function S.grey(lvl)
  if lvl <= 5 then return 0 end
  if lvl <= 39 then return lvl - 5 - math.floor(lvl / 10) end
  if lvl <= 59 then return lvl - 1 - math.floor(lvl / 5) end
  return lvl - 9
end

---------------------------------------------------------------------------------------------------
-- Deaths

function S.recent_deaths()
  local n, t = 0, now()
  for _, d in ipairs(S.deaths) do
    if t - d.at < S.death_window then n = n + 1 end
  end
  return n
end

-- The recent death within `death_radius` of point `p`, if any.
function S.death_near(p)
  local t = now()
  for _, d in ipairs(S.deaths) do
    if t - d.at < S.death_window and d.x and p.x and far(d, p) < S.death_radius then return d end
  end
end

-- Remember a death: where, at what level, and what was on us.
function S.died(killer)
  local pos = AgentPosition and AgentPosition()
  S.deaths[#S.deaths + 1] = { x = pos and pos.x, y = pos and pos.y, at = now(), level = level(), killer = killer }
  if #S.deaths > 10 then table.remove(S.deaths, 1) end
  note("died" .. (killer and (" to " .. killer) or ""))
end

-- A close call: a fight fled, or won with little left.
function S.scare(what)
  S.scares[#S.scares + 1] = { at = now(), what = what }
  if #S.scares > 10 then table.remove(S.scares, 1) end
  note("close call: " .. tostring(what))
end

function S.recent_scares()
  local n, t = 0, now()
  for _, s in ipairs(S.scares) do
    if t - s.at < S.death_window then n = n + 1 end
  end
  return n
end

function S.band()
  local lvl = level()
  local low = S.grey(lvl) + 1
  local high = lvl + S.above - S.death_penalty * S.recent_deaths() - S.recent_scares()
  -- Never below two under us: there is always something left that gives XP.
  return low, math.max(low, lvl - 2, high)
end

---------------------------------------------------------------------------------------------------
-- Before a pull

local RANKS = { elite = "an elite", rareelite = "a rare elite", worldboss = "a boss", rare = "a rare" }

-- Whether a unit is a mob for us at all, whatever our state: ok, else why not.
function S.fits(u)
  if u.kind ~= "unit" or u.dead then return false, "not a living mob" end
  if u.reaction ~= "hostile" and u.reaction ~= "neutral" then return false, "friendly" end
  if u.npc then return false, "an NPC with a service" end
  if u.tapped and not u.tapped_by_me then return false, "another player's" end
  if u.critter or u.creature_type == "Critter" then return false, "a critter" end
  if RANKS[u.classification or ""] then return false, RANKS[u.classification] end
  if not u.level then return false, "its level is unknown" end
  local low, high = S.band()
  if u.level < low then return false, "grey: no XP at level " .. u.level end
  if u.level > high then
    return false, string.format("level %d, above the level %d we pull now", u.level, high)
  end
  return true
end

-- A mob's aggro radius against us (yards): 20 at our level, a yard per level of difference.
function S.aggro(lvl)
  return math.max(5, math.min(45, 20 + ((lvl or level()) - level())))
end

-- How much fight a mob is, next to one of our level (1): less below us, more above.
function S.weight(lvl)
  local d = (lvl or level()) - level()
  if d >= 0 then return 1 + 0.25 * d end
  return math.max(0.25, 1 + 0.15 * d)
end

-- The living hostiles that would join a fight with `u`: within their aggro radius of where it
-- stands, or close enough to help it. Returns the count, the first one's name, and their weight.
function S.adds(u, list)
  local n, first, weight = 0, nil, 0
  for _, o in ipairs(list or seen(100)) do
    if o.guid ~= u.guid and o.kind == "unit" and not o.dead and o.reaction == "hostile" and o.x and not (o.critter or o.creature_type == "Critter")
      and not (o.tapped and not o.tapped_by_me) then
      local d = far(o, u)
      if d < S.assist or d < S.aggro(o.level) then
        n, first, weight = n + 1, first or o.name, weight + S.weight(o.level)
      end
    end
  end
  return n, first, weight
end

-- Whether a pull of `u` is too much at once with what would join: true and why.
function S.crowded(u, list)
  local n, first, weight = S.adds(u, list)
  if n == 0 or S.weight(u.level) + weight <= S.max_pull then return false end
  return true, string.format("%d more hostile%s would join (%s)", n, n == 1 and "" or "s", tostring(first))
end

-- Durability 0 on the main hand: the game refuses every swing.
function S.weapon_broken()
  if not GetInventoryItemDurability then return false end
  local cur, max = GetInventoryItemDurability(16)
  return cur ~= nil and max ~= nil and max > 0 and cur == 0
end

-- The health (a fraction) a fight with a mob of that level is started at: `pull_hp` for one of
-- our level, more for one above us, less for one below.
function S.health_for(lvl)
  return math.max(0.5, math.min(0.95, S.pull_hp * S.weight(lvl)))
end

-- Whether we are fit to start a fight (with unit `u`, if given): ok, else why not.
function S.ready(u)
  if S.weapon_broken() then return false, "the main hand is broken: repair first" end
  local hp, need = S.hp(), u and S.health_for(u.level) or S.pull_hp
  if hp < need then
    return false, string.format("health %d%%: rest first (%d%% for this one)", hp * 100, need * 100 + 0.5)
  end
  local mana = S.mana()
  if mana < S.pull_mana then return false, string.format("mana %d%%: drink first", mana * 100) end
  return true
end

local function attacking_us(u)
  local me = UnitGUID and UnitGUID("player")
  return me ~= nil and u.target == me and u.in_combat and not u.dead and u.reaction ~= "friendly"
end
S.attacking_us = attacking_us

-- Whether to start a fight with `u`: ok, else why not. `list` is an AgentNearby read to share.
function S.pull_ok(u, list)
  if not S.on or attacking_us(u) then return true end
  local ok, why = S.fits(u)
  if not ok then return false, why end
  ok, why = S.ready(u)
  if not ok then return false, why end
  local crowded, crowd = S.crowded(u, list)
  if crowded then return false, crowd end
  local d = u.x and S.death_near(u)
  if d then return false, "close to where we died" .. (d.killer and (" to " .. d.killer) or "") end
  return true
end

---------------------------------------------------------------------------------------------------
-- The bags

-- The first bag item whose use spell is `spell` ("Food", "Drink", "Healing Potion", "First Aid")
-- or whose name holds `name`, and which is off cooldown: bag, slot, name.
function S.item(spell, name)
  if not GetContainerNumSlots then return nil end
  for bag = 0, 4 do
    for slot = 1, (GetContainerNumSlots(bag) or 0) do
      local link = GetContainerItemLink(bag, slot)
      if link then
        local use = GetItemSpell and GetItemSpell(link)
        local item = GetItemInfo and GetItemInfo(link)
        local min_level = GetItemInfo and select(5, GetItemInfo(link)) or 0
        local hit = (spell and use == spell) or (name and item and item:find(name, 1, true))
        if hit and (min_level or 0) <= level() then
          local start, duration = 0, 0
          if GetContainerItemCooldown then start, duration = GetContainerItemCooldown(bag, slot) end
          if not start or start == 0 or start + (duration or 0) <= now() then return bag, slot, item end
        end
      end
    end
  end
end

-- How many bag items have that use spell (stacks counted in full).
function S.count(spell)
  local n = 0
  if not GetContainerNumSlots then return 0 end
  for bag = 0, 4 do
    for slot = 1, (GetContainerNumSlots(bag) or 0) do
      local link = GetContainerItemLink(bag, slot)
      if link and GetItemSpell and GetItemSpell(link) == spell then
        local _, count = GetContainerItemInfo(bag, slot)
        n = n + (count or 1)
      end
    end
  end
  return n
end

-- Drink a healthstone, else a healing potion. Returns what was used.
function S.heal_item()
  local bag, slot, item = S.item(nil, "Healthstone")
  if not bag then bag, slot, item = S.item("Healing Potion") end
  if not bag then return nil end
  UseContainerItem(bag, slot)
  note("used " .. tostring(item))
  return item
end

local function aura(unit, name, filter)
  if not UnitAura then return false end
  for i = 1, 40 do
    local n = UnitAura(unit, i, filter)
    if not n then return false end
    if n == name then return true end
  end
  return false
end

-- Rest between fights: food for health and drink for mana from the bags, else a bandage, else
-- sit. Returns what it did, for the log.
function S.eat()
  local did = {}
  if S.hp() < 0.95 and not aura("player", "Food", "HELPFUL") then
    -- Our own cooking first, then the best food we may eat (Professions); else the first found.
    local bag, slot, item
    if Professions and Professions.best_food then bag, slot, item = Professions.best_food() end
    if not bag then bag, slot, item = S.item("Food") end
    if not bag and not aura("player", "Recently Bandaged", "HARMFUL") then bag, slot, item = S.item("First Aid") end
    if bag then
      UseContainerItem(bag, slot)
      did[#did + 1] = item or "food"
    end
  end
  if S.mana() < 0.95 and not aura("player", "Drink", "HELPFUL") then
    local bag, slot, item = S.item("Drink")
    if bag then
      UseContainerItem(bag, slot)
      did[#did + 1] = item or "drink"
    end
  end
  if #did == 0 and DoEmote then
    -- The emote, not the sit/stand key: it never stands a sitting character up again.
    DoEmote("SIT")
    S.sitting = true
    did[1] = "sat down"
  end
  return table.concat(did, ", ")
end

-- Back on our feet if we sat down to rest: a fight is not swung, nor a corpse looted, sitting.
function S.stand()
  if S.sitting and DoEmote then DoEmote("STAND") end
  S.sitting = nil
end

---------------------------------------------------------------------------------------------------
-- In a fight

-- Health samples over the last `window` seconds: ours, and each attacker's by guid.
local samples = {}
local cornered_until = 0
local fight_began

function S.fight_start()
  samples, fight_began = {}, now()
end

-- A fight won: what we dealt per second joins the running average the next fights are judged by.
function S.fight_won(health, seconds, name)
  if S.hp() < S.scare_below then S.scare(string.format("%s left us at %d%%", tostring(name), S.hp() * 100)) end
  if not health or health <= 0 or not seconds or seconds < 2 then return end
  -- A mob with a fraction of our health says nothing about a real fight.
  if health < (UnitHealthMax("player") or 0) * 0.3 then return end
  local rate = health / seconds
  S.dps = S.dps and (S.dps * 0.7 + rate * 0.3) or rate
end

-- Everything in combat with us, and the fight's own target.
function S.attackers(target_guid, list)
  local out = {}
  for _, u in ipairs(list or seen(40)) do
    if u.kind ~= "gameobject" and not u.dead and (attacking_us(u) or u.guid == target_guid) then out[#out + 1] = u end
  end
  return out
end

-- How the fight is going, from the health lost on each side over the last seconds:
-- { attackers, hp, incoming, outgoing (health per second), ours, theirs (seconds each side
-- lasts at those rates), losing, decisive, seconds (measured) }. Until the window is full, what
-- we deal is taken from the fights won before (`dps`): a slow weapon lands nothing for seconds.
-- The same while the target is out of reach (`in_reach` false): closing on it, we deal nothing
-- yet, which says nothing about the fight.
function S.assess(target_guid, list, in_reach)
  local t = now()
  local foes = S.attackers(target_guid, list)
  local sample = { t = t, hp = UnitHealth("player") or 0, foes = {} }
  local remaining, outmatched = 0, #foes >= 2
  for _, u in ipairs(foes) do
    sample.foes[u.guid] = u.health or 0
    remaining = remaining + (u.health or 0)
    if (u.level or 0) >= level() + 2 then outmatched = true end
  end
  samples[#samples + 1] = sample
  while #samples > 1 and t - samples[2].t >= S.window do table.remove(samples, 1) end
  local first = samples[1]
  local seconds = t - first.t
  local a = { attackers = #foes, hp = S.hp(), incoming = 0, outgoing = 0, seconds = seconds,
    ours = math.huge, theirs = math.huge, losing = false, decisive = false }
  if seconds < 3 then return a end
  a.incoming = math.max(0, first.hp - sample.hp) / seconds
  -- Only what stood on both ends of the window: a mob that joined did not gain us damage.
  local dealt = 0
  for guid, hp in pairs(first.foes) do
    dealt = dealt + math.max(0, hp - (sample.foes[guid] or 0))
  end
  a.outgoing = dealt / seconds
  local young = seconds < S.window - 1
    or (dealt == 0 and (in_reach == false or t - (fight_began or first.t) < S.window + 4))
  -- In reach all this time and nothing landed: our swings are not going out (turned away, the
  -- attack not started). That is Combat's to put right, not a fight lost: judge it by what we
  -- usually deal, and leave only once we are really hurt.
  a.stalled = dealt == 0 and not young
  if young or a.stalled then
    if not S.dps then return a end
    a.outgoing = S.dps
  end
  if a.incoming > 0 then a.ours = sample.hp / a.incoming end
  a.theirs = a.outgoing > 0 and remaining / a.outgoing or math.huge
  if remaining <= 0 then a.theirs = 0 end
  a.losing = a.incoming > 0 and a.theirs > a.ours
  -- Sure enough to leave on: measured over the whole window, or (on the first seconds' reading)
  -- against more than one mob or one well above us. One hard hit from an even mob is not it.
  a.decisive = a.losing and a.theirs > a.ours * S.decisive and (not young or outmatched) and not a.stalled
  return a
end

-- The class's emergency buttons, in order; each is cast when known, ready and its `when` holds.
-- `a` is Survive.assess's reading.
S.EMERGENCY = {
  WARRIOR = {
    { spell = "Last Stand", when = function(a) return a.hp < 0.35 end },
    { spell = "Shield Wall", when = function(a) return a.hp < 0.35 end },
    { spell = "Enraged Regeneration", when = function(a) return a.hp < 0.5 end },
    { spell = "Intimidating Shout", when = function(a) return a.hp < 0.35 and a.attackers >= 2 end },
    { spell = "Retaliation", when = function(a) return a.attackers >= 2 end },
  },
  PALADIN = {
    { spell = "Lay on Hands", when = function(a) return a.hp < 0.2 end },
    { spell = "Divine Protection", when = function(a) return a.hp < 0.4 end },
    { spell = "Hammer of Justice", target = true, when = function(a) return a.hp < 0.4 end },
  },
  ROGUE = {
    { spell = "Evasion", when = function(a) return a.hp < 0.6 or a.attackers >= 2 end },
  },
  HUNTER = {
    { spell = "Deterrence", when = function(a) return a.hp < 0.4 end },
  },
  MAGE = {
    { spell = "Frost Nova", target = true, when = function(a) return a.hp < 0.6 end },
    { spell = "Mana Shield", when = function(a) return a.hp < 0.35 end },
    { spell = "Ice Block", when = function(a) return a.hp < 0.2 end },
  },
  PRIEST = {
    { spell = "Power Word: Shield", when = function(a) return a.hp < 0.6 and not aura("player", "Weakened Soul", "HARMFUL") end },
    { spell = "Desperate Prayer", when = function(a) return a.hp < 0.35 end },
    { spell = "Psychic Scream", when = function(a) return a.hp < 0.4 and a.attackers >= 2 end },
  },
  WARLOCK = {
    { spell = "Death Coil", target = true, when = function(a) return a.hp < 0.35 end },
  },
  SHAMAN = {
    { spell = "Stoneclaw Totem", when = function(a) return a.attackers >= 2 end },
  },
  DRUID = {
    { spell = "Barkskin", when = function(a) return a.hp < 0.5 end },
  },
  DEATHKNIGHT = {
    { spell = "Icebound Fortitude", when = function(a) return a.hp < 0.4 end },
  },
  -- Racials, tried after the class's own.
  ANY = {
    { spell = "Gift of the Naaru", when = function(a) return a.hp < 0.5 end },
    { spell = "Stoneform", when = function(a) return a.hp < 0.4 end },
    { spell = "War Stomp", target = true, when = function(a) return a.hp < 0.5 and a.attackers >= 2 end },
  },
}

-- What slows or stops the mob so a run gets clear, by class (the first one ready is cast).
S.SNARE = {
  WARRIOR = { "Hamstring", "Piercing Howl", "Intimidating Shout" },
  PALADIN = { "Hammer of Justice" },
  HUNTER = { "Concussive Shot", "Wing Clip" },
  ROGUE = { "Gouge", "Sprint" },
  MAGE = { "Frost Nova" },
  PRIEST = { "Psychic Scream" },
  SHAMAN = { "Earthbind Totem", "Frost Shock" },
  DRUID = { "Nature's Grasp" },
  WARLOCK = { "Death Coil", "Howl of Terror" },
  DEATHKNIGHT = { "Chains of Ice" },
}

local function class()
  local _, token = UnitClass("player")
  return token or ""
end

local function castable(spell)
  return Rotation ~= nil and Rotation.castable ~= nil and Rotation.castable(spell)
end

local function emergency(a)
  for _, list in ipairs({ S.EMERGENCY[class()] or {}, S.EMERGENCY.ANY }) do
    for _, e in ipairs(list) do
      if castable(e.spell) then
        local ok, yes = pcall(e.when, a)
        if ok and yes and Rotation.cast(e.spell, not e.target) then
          note("cast " .. e.spell)
          return e.spell
        end
      end
    end
  end
end

-- The snare that could be cast now, if any.
function S.snare()
  for _, spell in ipairs(S.SNARE[class()] or {}) do
    if castable(spell) then return spell end
  end
end

-- Whether a run from this fight can be survived: what they deal while at our heels is less than
-- what we have left.
function S.can_escape(a)
  local seconds = S.snare() and S.snared_seconds or S.run_seconds
  return a.incoming * seconds < (UnitHealth("player") or 0) * 0.9
end

-- Where to run: `run_yards` off, away from the attackers and clear of every other hostile's
-- aggro, on a route the navmesh has (AgentPath) when it can say.
function S.escape_point(foes, list)
  local me = AgentPosition and AgentPosition()
  if not me then return nil end
  list = list or seen(100)
  local cx, cy, n = 0, 0, 0
  local chasing = {}
  for _, u in ipairs(foes or {}) do
    if u.x then cx, cy, n = cx + u.x, cy + u.y, n + 1 end
    chasing[u.guid] = true
  end
  local away
  if n > 0 then
    cx, cy = cx / n, cy / n
    away = math.atan2(me.y - cy, me.x - cx)
  end
  local best, best_score
  for i = 0, 15 do
    local angle = i * math.pi / 8
    local p = { x = me.x + math.cos(angle) * S.run_yards, y = me.y + math.sin(angle) * S.run_yards, z = me.z }
    -- Clear of what is not chasing us yet: the nearest such hostile's aggro edge, to 30 yards.
    local clear = 30
    for _, o in ipairs(list) do
      if o.kind == "unit" and not o.dead and o.reaction == "hostile" and o.x and not chasing[o.guid] then
        clear = math.min(clear, far(o, p) - S.aggro(o.level))
        -- And not past one on the way there.
        local mid = { x = (me.x + p.x) / 2, y = (me.y + p.y) / 2 }
        clear = math.min(clear, far(o, mid) - S.aggro(o.level))
      end
    end
    local score = clear
    if away then
      local turn = math.abs((angle - away + math.pi) % (2 * math.pi) - math.pi)
      score = score + 20 * math.cos(turn)
    end
    if not best_score or score > best_score then
      local route = AgentPath and AgentPath(p.x, p.y)
      if not AgentPath or (route and route.complete and (route.length or 0) < S.run_yards * 2) then
        best, best_score = p, score
      end
    end
  end
  return best
end

-- A run that did not shake them: no more fleeing for a while, the fight is fought out.
function S.cornered(seconds)
  cornered_until = now() + (seconds or 60)
  note("could not get away: fighting it out")
end

-- One tick of a fight. Returns "flee" when Combat should run, else what it used (for the state
-- line), else nil. `target` is the fight's unit (an AgentNearby entry).
function S.fight_tick(target)
  if not S.on then return nil end
  local list = seen(100)
  local a = S.assess(target and target.guid, list, not target or (target.distance or 0) <= 8)
  S.last = a
  local target_pct = target and fraction(target.health, target.health_max) or 1
  -- One mob nearly dead is finished, not fled from.
  local nearly_won = a.attackers <= 1 and target_pct < 0.15
  if nearly_won then return nil end
  local broken = S.weapon_broken()
  local losing = a.losing or broken
  if a.hp < S.critical or (a.hp < S.potion_below and losing) then
    local used = S.heal_item()
    if used then return "used " .. used end
  end
  if a.hp < 0.6 or a.attackers >= 2 then
    local cast = emergency(a)
    if cast then return cast end
  end
  if now() < cornered_until then return nil end
  local lost = a.decisive or broken or ((losing or a.stalled) and a.hp < S.flee_below)
  if lost and a.seconds >= 3 and S.can_escape(a) then
    note(string.format("fleeing %s (%d yd) at %d%%: %d on us, %.0f s left to their %.0f", tostring(target and target.name),
      target and target.distance or -1, a.hp * 100, a.attackers, a.ours, a.theirs))
    return "flee"
  end
  return nil
end

local frame = CreateFrame("Frame")
frame:RegisterEvent("PLAYER_DEAD")
frame:SetScript("OnEvent", function()
  local killer
  for _, u in ipairs(S.attackers(Combat and Combat.active, seen(40))) do killer = killer or u.name end
  S.died(killer)
end)

return S
