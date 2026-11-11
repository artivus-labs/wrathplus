-- wp.goals: what the agent is working toward, and the next thing to do about it, so it is never
-- idle. Loaded for every agent as the global `Goals`. The planner sets goals and teaches actions;
-- a small GOAP planner here chains actions toward the top goal, and a runner starts the next one
-- whenever nothing else runs. Everything it reads is the client's own: the quest log, what is in
-- sight, the bags, the zones' levels from the client's tables.
--
-- Goals:
--   Goals.push{ name, why, priority = 50, done = fn | "lua", want = { fact = value }, standing,
--               data = {...} }
--       Replaces the goal of that name. `want` is the world state that satisfies it, what plans
--       chain toward; without one it wants the fact named after the goal (teach a fact or an
--       action with that effect). `done` (a function, or Lua source as the planner tools send it)
--       returns finished, ok: the goal is then signalled AGENT_GOAL_DONE(name, ok) and leaves the
--       plan. A standing goal never finishes.
--   Goals.drop(name)   Goals.list() → { { name, why, priority, standing, status, plan, note }, ... }
--       highest first; status is "active", "blocked" (no plan reaches it now: AGENT_GOAL_BLOCKED
--       (name, why) once as it blocks), "done" or "failed".
--   `progress` is always there, at the lowest priority: hand in, pick up, work objectives, train,
--       sell and repair, rest, else grind, else look further afield or travel to a better zone.
-- Facts, the planner's world state, read fresh each pass (Goals.facts(); Goals.fact(name, fn) adds
--   one): quest_ready, objectives_open, quest_giver_near, hand_in_near, bags_full (fewer free
--   slots than Upkeep.full_below), needs_repair, can_train, trainer_known, vendor_known,
--   repair_known, food_vendor_known (seen this session, and where: Goals.places), hp_low,
--   mana_low (below what a fight is started at), gear_broken
--   (the main hand cannot swing), pull_ok (fit to start a fight: none of those three),
--   needs_food (nothing to eat, or to drink for a mana user, and money for some), recent_deaths
--   (in the last ten minutes: each lowers the levels pulled, Survive.band), mobs_for_xp (a mob in
--   sight that gives XP and is a safe pull: Survive.pull_ok), zone_fit ("low": the zone is below
--   us, "ok", "high": above us), in_combat, dead. wp.professions adds can_cook, can_bandage,
--   food_low, bandages_low, can_skill_up and profession_to_learn.
-- Actions:
--   Goals.action{ name, why, pre = { fact = value } | fn(facts), effects = { fact = value },
--                 cost = n | fn(facts), run = fn(args), done = fn() → finished, ok, why, timeout,
--                 stop = fn(why) }
--       `args` is { goal, data, facts }; `run` may set args.subject (the quest objective, the zone)
--       so a failure backs off that one, not the whole action; `stop` undoes what an action that
--       ended badly left going (its fight, its walk). Built in: hand_in_quests,
--       pick_up_quests, work_objective, train, sell_and_repair, buy_food, rest, grind, seek_mobs,
--       travel_to_zone (and wp.professions' learn_professions, craft). Fights are only started
--       when Survive.pull_ok says so (grind and work_objective take the nearest safe mob, and wait
--       for or give up on an unsafe one),
--       and whatever the plan, the runner rests first when hurt.
--       A plan is the cheapest chain of actions (A* over the facts) from now to
--       the goal's `want`; plans are made again on the agent's signals (an activity's end, a
--       kill, a quest visit) and the client's (level up, quest log, death, zone), else every
--       `replan_every` seconds.
-- Runner (`Goals.on`, off until the agent turns it on): with no Agent activity running, no hold,
--   not fighting or dead, no cinematic playing (AgentCinematic), the next action of the top goal
--   that has a plan runs AS an Agent activity named `goal:<action>`, so the fast tier and the
--   status line see it. A failed action
--   is AGENT_ACTION_FAILED(goal, action, why) and waits `retry_after` seconds before it is tried
--   again, doubling on each failure in a row (an interruption, a death or a hold is no failure);
--   one that failed within `quick_fail` seconds of starting waits even when nothing else is left.
--   The fast tier is offered goal actions on the same terms: not while fighting or dead, and not
--   while one backs off.
--   Between two actions Agent.activity() is nil for a moment: the runner looks every half second,
--   and on the frame after an action ends well; a fight still winding down (a kill being looted
--   after its quest credit ended the action: a second or two), a hold or its grace, a cinematic,
--   and an instant action (no `done`) read nil too. `Goals.state` says which, so poll that (or
--   wait on the AGENT_ signals) rather than take one nil for idleness.
-- Holds: Goals.hold{ reason, seconds } stops the runner (and its running action) for that long;
--   there is no unlimited form. On expiry AGENT_HOLD_EXPIRED(reason, waited) is signalled and the
--   runner waits `grace` seconds more for a Goals.hold (keep waiting: `waited` keeps counting while
--   the reason stays the same) or Goals.release(). Goals.holding() → reason, waited, remaining.
-- Zones: Goals.zones() → the zones on this continent we may level in (AgentZones: our faction's
--   and contested, no capitals), each with `fit` against our level and `distance`. The planner can
--   push its own travel goal: a goal's `data.zone` names the zone travel_to_zone heads for. A
--   data.zone that is not in that list, or whose fit is "high", is refused by Goals.push with the
--   zones that fit. An action the planner taught (not built in) that walks into a zone above us
--   is stopped there and fails with the same list.
--
-- Places: Goals.places[kind] lists the NPCs seen (kinds trainer, our class's; vendor, every
--   merchant, which buys any item, quartermasters too; repair), up to `places_max` each, and an
--   action goes to the nearest. Goals.learn_places{ { kind, name, x, y [, z, map, zone, guid] }, ... }
--   adds places from elsewhere (the realm's own list of its NPCs).
-- Keep: Goals.keep{ "name", ... } adds items a full bag never sells at a merchant (Upkeep.keep);
--   selling that still leaves the bags full fails sell_and_repair with what fills them, and it
--   backs off like any failure.
-- Walks the planner starts by hand (AgentMoveTo, Move.to, AgentInteract from afar, in its lua
--   tool, outside any activity) while the runner is on: the runner holds while the walk lasts
--   (at most `walk_hold_cap` seconds), the goal action it pre-empted ends as interrupted, and the
--   tool's output says which action and where it was going. A stop or a sit by hand (AgentStop,
--   SitStandOrDescendStart, DoEmote("sit")) holds it `stop_hold` seconds the same way. The
--   runner's own walks are not held.
--
-- `Goals.state` says what the runner is doing; `Goals.log` keeps its last few lines.

local G = {
  on = false, goals = {}, actions = {}, places = {}, log = {}, state = nil,
  replan_every = 5, retry_after = 20, retry_cap = 300, grace = 3, max_steps = 5,
  -- An action that fails within this many seconds of starting is not tried again before its
  -- backoff is over, even by the standing goal that never idles.
  quick_fail = 2,
  -- Health below this out of combat is hp_low; mana below `mana_below` is mana_low; at or above
  -- `rest_until` health is never low (below it, it is where the mobs here want more). Resting
  -- goes on to full health and mana.
  rest_below = 0.7, mana_below = 0.4, rest_until = 0.9,
  -- Mobs this many levels above us still count when reading how a zone suits us (zone_fit).
  above = 2,
  -- A walk that ends within this many yards of a quest's hand-in point has arrived at it.
  hand_in_reach = 60,
  -- An objective's mob that is no safe pull is waited on this long before the objective is left;
  -- an objective whose area is too dangerous is left alone for `danger_wait`; food is bought
  -- with at least this much money (copper).
  unsafe_wait = 20, danger_wait = 600, food_money = 50,
  -- NPCs remembered per kind, and the longest a walk of the planner's holds the runner.
  places_max = 8, walk_hold_cap = 300,
  -- How long the runner holds after the planner stops the character or sits it down by hand.
  stop_hold = 30,
  failures = {}, current = nil, held = nil, after = nil,
}

local facts_fns, fact_order, fact_errors = {}, {}, {}
-- What the fast tier is offered now (`goal:<action>` intent -> { goal, text }), and the intent
-- each action runs as (set up with the runner, below).
local offered, register = {}, nil
local dirty, planned_at, last_facts = true, nil, {}
-- An activity just ended well: the runner looks again on the next frame, not the next half second.
local kick = false

local function now() return GetTime() end

local function note(text)
  G.log[#G.log + 1] = string.format("%.1f %s", now(), text)
  if #G.log > 20 then table.remove(G.log, 1) end
end

local function signal(name, ...)
  if Agent and Agent.signal then Agent.signal(name, ...) end
end

local function valid_name(name)
  return type(name) == "string" and name:match("^[a-z][a-z0-9_]*$") ~= nil
end

-- A function from Lua source as the planner sends it: an expression, else a chunk.
local function compile(code, what)
  if type(code) ~= "string" then return code end
  local fn = loadstring("return " .. code, "=" .. what)
  if not fn then
    local err
    fn, err = loadstring(code, "=" .. what)
    assert(fn, what .. ": " .. tostring(err))
  end
  return fn
end

local function far(a, b) return math.sqrt((a.x - b.x) ^ 2 + (a.y - b.y) ^ 2) end
local function me() return AgentPosition and AgentPosition() end
local function seen(yards) return AgentNearby and AgentNearby(yards) or {} end
local function level() return UnitLevel("player") or 1 end

local function ratio(value, max)
  max = max or 0
  if max <= 0 then return 1 end
  return (value or 0) / max
end

-- 3.3.5's grey level: mobs at or below it give no XP.
local function grey(lvl)
  if lvl <= 5 then return 0 end
  if lvl <= 39 then return lvl - 5 - math.floor(lvl / 10) end
  if lvl <= 59 then return lvl - 1 - math.floor(lvl / 5) end
  return lvl - 9
end

-- Whether a unit is worth fighting for XP: alive, not friendly, not someone else's tap, no NPC
-- service, no elite, rare or critter, and inside the levels we pull (Survive.fits, Survive.band).
function G.fits(u)
  if Survive then return (Survive.fits(u)) end
  local lvl = level()
  return u.kind == "unit" and not u.dead and (u.reaction == "hostile" or u.reaction == "neutral")
    and not (u.tapped and not u.tapped_by_me) and not u.npc and u.creature_type ~= "Critter"
    and u.level ~= nil and u.level > grey(lvl) and u.level <= lvl + G.above
end

-- Whether to start a fight with a unit now (Survive.pull_ok): ok, else why not.
function G.safe(u, list)
  if Survive then return Survive.pull_ok(u, list) end
  if not G.fits(u) then return false, "not a mob for us" end
  return true
end

-- Where mobs that fit were seen (one spot per 20 yards), for seek_mobs to walk back to.
local sightings = {}

function G.sighted(u)
  for _, s in ipairs(sightings) do
    if far(s, u) < 20 then
      s.at = now()
      return
    end
  end
  sightings[#sightings + 1] = { x = u.x, y = u.y, at = now() }
  if #sightings > 20 then table.remove(sightings, 1) end
end

-- Failures in a row back an action (or one subject of it) off: `retry_after` doubled each time.
function G.cooling(key)
  local f = G.failures[key]
  return f ~= nil and now() < f.until_
end

---------------------------------------------------------------------------------------------------
-- Goals

local function goal_text(goal)
  local parts = {}
  for k, v in pairs(goal.want) do parts[#parts + 1] = k .. " = " .. tostring(v) end
  table.sort(parts)
  return table.concat(parts, ", ")
end

-- Why a goal may not send us to `name`, or nil: it must be a zone we may level in on this
-- continent, and not one above us. Levels come from the client's tables (Goals.zones()), never
-- from anyone's memory of them. No zone table (no client data) is nothing to check against.
local function zone_refusal(name)
  local zones = G.zones()
  if #zones == 0 then return nil end
  local target, fits = nil, {}
  for _, z in ipairs(zones) do
    if z.name == name then target = z end
    if z.fit == "ok" then
      fits[#fits + 1] = string.format("%s (%d-%d%s)", z.name, z.min, z.max, z.distance and (", " .. z.distance .. " yd") or "")
    end
  end
  local fit = "Zones that fit level " .. level() .. ": " .. (#fits > 0 and table.concat(fits, ", ") or "none on this continent")
    .. ". Zone levels come from Goals.zones(), not from memory."
  if not target then return "no zone named " .. name .. " to level in on this continent. " .. fit end
  if target.fit == "high" then
    return string.format("%s is levels %d-%d, above you at level %d: a goal that travels there is refused. %s",
      name, target.min, target.max, level(), fit)
  end
end

function G.push(spec)
  assert(type(spec) == "table" and valid_name(spec.name), "goal names are snake_case words")
  local zone = spec.data and spec.data.zone
  if zone ~= nil then
    local why = zone_refusal(tostring(zone))
    if why then error("goal " .. spec.name .. ": " .. why, 0) end
  end
  local done = spec.done and compile(spec.done, "goal " .. spec.name)
  assert(done == nil or type(done) == "function", "goal " .. spec.name .. ": done is a function")
  local goal = {
    name = spec.name, why = spec.why, priority = spec.priority or 50, done = done,
    want = spec.want or { [spec.name] = true }, standing = spec.standing == true,
    data = spec.data or {}, status = "active", plan = {}, since = now(),
  }
  G.goals[spec.name] = goal
  dirty = true
  return goal
end

function G.drop(name)
  if name == "progress" then return false, "progress is always there" end
  if not G.goals[name] then return false, "no goal " .. tostring(name) end
  G.goals[name] = nil
  dirty = true
  return true
end

local function ordered()
  local out = {}
  for _, goal in pairs(G.goals) do out[#out + 1] = goal end
  table.sort(out, function(a, b)
    if a.priority ~= b.priority then return a.priority > b.priority end
    return a.name < b.name
  end)
  return out
end

---------------------------------------------------------------------------------------------------
-- Facts

function G.fact(name, fn)
  assert(valid_name(name), "fact names are snake_case words")
  fn = compile(fn, "fact " .. name)
  assert(type(fn) == "function", "fact " .. name .. " needs a function")
  if not facts_fns[name] then fact_order[#fact_order + 1] = name end
  facts_fns[name] = fn
  dirty = true
end

-- One pass's shared reads, so a dozen facts look at the world once.
local pass = {}

local function nearby()
  pass.nearby = pass.nearby or seen(100)
  return pass.nearby
end

local function quests()
  pass.quests = pass.quests or (Quests and Quests.list and Quests.list()) or {}
  return pass.quests
end

function G.facts()
  pass = {}
  local out = {}
  for _, name in ipairs(fact_order) do
    local ok, value = pcall(facts_fns[name])
    if ok then
      out[name], fact_errors[name] = value, nil
    else
      fact_errors[name] = tostring(value)
    end
  end
  last_facts = out
  return out, fact_errors
end

local FOOD_TITLES = { "food", "butcher", "baker", "bread", "meat", "cheese", "fruit", "fish", "mushroom",
  "grocer", "cook", "bartender", "barmaid", "tavern", "drink", "waitress" }

local function sells_food(title)
  title = (title or ""):lower()
  for _, word in ipairs(FOOD_TITLES) do
    if title:find(word, 1, true) then return true end
  end
  return false
end


local function same_place(a, b)
  if a.guid and b.guid then return a.guid == b.guid end
  return a.name == b.name and a.map == b.map and far(a, b) < 40
end

-- Remember a place of a kind: the same NPC again moves to the front, the oldest beyond
-- `places_max` is forgotten.
local function learn(kind, p)
  local list = G.places[kind] or {}
  G.places[kind] = list
  for i = #list, 1, -1 do
    if same_place(list[i], p) then table.remove(list, i) end
  end
  table.insert(list, 1, p)
  while #list > G.places_max do table.remove(list) end
end

-- An NPC that was not where it stood, under every kind.
local function forget(p)
  for _, list in pairs(G.places) do
    for i = #list, 1, -1 do
      if same_place(list[i], p) then table.remove(list, i) end
    end
  end
end

-- Remember where the NPCs we need stood, as a player remembers the trainer's house. A smith who
-- repairs is a vendor too: every merchant buys any item.
local function remember(list)
  local class = UnitClass and UnitClass("player")
  local zone = GetRealZoneText and GetRealZoneText()
  local pos = me()
  for _, u in ipairs(list) do
    if u.kind == "unit" and not u.dead and u.npc then
      local place = { guid = u.guid, name = u.name, x = u.x, y = u.y, z = u.z, zone = zone,
        map = pos and pos.map, at = now() }
      for _, s in ipairs(u.npc) do
        if s == "classtrainer" and class and u.title and u.title:lower():find(class:lower(), 1, true) then
          learn("trainer", place)
        elseif s == "repair" or s == "vendor" then
          learn(s, place)
          if s == "repair" then learn("vendor", place) end
        end
        -- Innkeepers sell food and drink; so do the vendors whose title says so.
        if s == "innkeeper" or (s == "vendor" and sells_food(u.title)) then learn("food", place) end
      end
    end
  end
end

-- Places from elsewhere: { { kind = "vendor" | "repair" | "trainer" | ..., name, x, y [, z, map,
-- zone, guid] }, ... }. A repair NPC is a vendor too. Returns how many were taken.
function G.learn_places(list)
  assert(type(list) == "table", "learn_places takes a list of places")
  local n = 0
  for _, p in ipairs(list) do
    assert(type(p) == "table" and valid_name(p.kind) and type(p.name) == "string" and tonumber(p.x) and tonumber(p.y),
      "a place is { kind, name, x, y [, z, map, zone, guid] }")
    local place = { guid = p.guid, name = p.name, x = tonumber(p.x), y = tonumber(p.y), z = tonumber(p.z),
      map = p.map, zone = p.zone, at = now() }
    learn(p.kind, place)
    if p.kind == "repair" then learn("vendor", place) end
    n = n + 1
  end
  dirty = true
  return n
end

-- The nearest remembered place of a kind on this continent, else nil.
local function place(kind)
  local pos, best, best_d = me(), nil, math.huge
  for _, p in ipairs(G.places[kind] or {}) do
    if not (pos and p.map and pos.map ~= p.map) then
      local d = pos and far(pos, p) or 0
      if d < best_d then best, best_d = p, d end
    end
  end
  return best
end

-- A giver in reach marked for us, leaving out those a visit just found had nothing (Quests.skipping).
local function marker(kind)
  for _, u in ipairs(nearby()) do
    if u.quest == kind and not u.dead and u.distance <= ((Quests and Quests.reach) or 30)
      and not (Quests and Quests.skipping and Quests.skipping(u.guid)) then
      return u
    end
  end
end

local function free_slots()
  if not GetContainerNumFreeSlots then return nil end
  local free = 0
  for bag = 0, 4 do free = free + (GetContainerNumFreeSlots(bag) or 0) end
  return free
end

-- The zone we stand in, from the client's zone list.
local function current_zone()
  local name = GetRealZoneText and GetRealZoneText()
  for _, z in ipairs(AgentZones and AgentZones() or {}) do
    if z.name == name then return z end
  end
end

local function median(list)
  if #list == 0 then return nil end
  table.sort(list)
  return list[math.floor((#list + 1) / 2)]
end

-- How the zone suits our level: what is in sight first (three or more mobs are evidence), else
-- the zone's band from the client's tables.
local function zone_fit()
  local lvl, levels = level(), {}
  for _, u in ipairs(nearby()) do
    -- Critters are level 1 wherever they hop: they say nothing about the zone.
    if u.kind == "unit" and not u.dead and u.level and (u.reaction == "hostile" or u.reaction == "neutral")
      and not u.npc and u.creature_type ~= "Critter" then
      levels[#levels + 1] = u.level
    end
  end
  local here
  if #levels >= 3 then
    here = median(levels)
  else
    local z = current_zone()
    if not z then return "ok" end
    if lvl > z.max + 1 then return "low" end
    if lvl < z.min - 1 then return "high" end
    return "ok"
  end
  if here <= grey(lvl) then return "low" end
  if here > lvl + G.above + 1 then return "high" end
  return "ok"
end

local BUILT_IN_FACTS = {
  { "in_combat", function() return UnitAffectingCombat("player") and true or false end },
  { "dead", function() return UnitIsDeadOrGhost("player") and true or false end },
  { "hp_low", function()
    local hp = ratio(UnitHealth("player"), UnitHealthMax("player"))
    if hp < G.rest_below then return true end
    if not Survive or hp >= G.rest_until then return false end
    -- Above the mark, but every mob we could pull here wants more health than we have (they
    -- are above our level): rest for them too.
    local some = false
    for _, u in ipairs(nearby()) do
      if G.fits(u) and not Survive.crowded(u, nearby()) then
        if hp >= Survive.health_for(u.level) then return false end
        some = true
      end
    end
    return some
  end },
  { "mana_low", function()
    if UnitPowerType and UnitPowerType("player") ~= 0 then return false end
    return ratio(UnitPower("player", 0), UnitPowerMax("player", 0)) < G.mana_below
  end },
  { "quest_ready", function()
    for _, q in ipairs(quests()) do if q.complete then return true end end
    return false
  end },
  { "objectives_open", function()
    for _, q in ipairs(quests()) do
      if not q.complete then
        for _, o in ipairs(q.objectives) do if not o.done then return true end end
      end
    end
    return false
  end },
  { "quest_giver_near", function() return marker("available") ~= nil end },
  { "hand_in_near", function() return marker("reward") ~= nil end },
  { "bags_full", function()
    local free = free_slots()
    return free ~= nil and free < ((Upkeep and Upkeep.full_below) or 2)
  end },
  { "needs_repair", function()
    if not GetInventoryItemDurability then return false end
    for slot = 1, 18 do
      local cur, max = GetInventoryItemDurability(slot)
      if cur and max and max > 0 and cur / max < 0.25 then return true end
    end
    return false
  end },
  { "can_train", function()
    return not (Upkeep and Upkeep.train == false) and level() > ((Upkeep and Upkeep.trained_level) or 1)
  end },
  { "trainer_known", function()
    remember(nearby())
    return place("trainer") ~= nil
  end },
  { "vendor_known", function() return place("vendor") ~= nil end },
  -- Repairs need a repair NPC: any vendor buys junk, but only some fix gear.
  { "repair_known", function() return place("repair") ~= nil end },
  { "gear_broken", function() return Survive ~= nil and Survive.weapon_broken() end },
  { "pull_ok", function() return Survive == nil or (Survive.ready()) end },
  { "recent_deaths", function() return Survive and Survive.recent_deaths() or 0 end },
  { "food_vendor_known", function() return place("food") ~= nil end },
  { "needs_food", function()
    if not Survive or (GetMoney and GetMoney() or 0) < G.food_money then return false end
    local mana_user = UnitPowerType ~= nil and UnitPowerType("player") == 0
    -- No food, unless we can cook some from the bags (wp.professions' craft makes it).
    local cook = Professions and Professions.known().Cooking and Professions.makeable("food") > 0
    return (Survive.count("Food") == 0 and not cook) or (mana_user and Survive.count("Drink") == 0)
  end },
  { "mobs_for_xp", function()
    local any = false
    for _, u in ipairs(nearby()) do
      if G.fits(u) then
        G.sighted(u)
        -- Our own state is the plan's business (rest, repair): here, only whether the mob is.
        if not any and not (Survive and Survive.crowded(u, nearby()))
          and not (Survive and u.x and Survive.death_near(u)) then
          any = true
        end
      end
    end
    return any
  end },
  { "zone_fit", zone_fit },
}
for _, f in ipairs(BUILT_IN_FACTS) do G.fact(f[1], f[2]) end

---------------------------------------------------------------------------------------------------
-- Actions and planning

function G.action(spec)
  assert(type(spec) == "table" and valid_name(spec.name), "action names are snake_case words")
  assert(type(spec.run) == "function", "action " .. spec.name .. " needs run")
  spec.effects = spec.effects or {}
  G.actions[spec.name] = spec
  register(spec)
  dirty = true
  return spec
end

function G.remove_action(name)
  G.actions[name] = nil
  if Agent and Agent.intents then Agent.intents["goal:" .. name] = nil end
  dirty = true
end

local function satisfied(state, want)
  for k, v in pairs(want) do
    if state[k] ~= v then return false end
  end
  return true
end

local function applicable(action, state)
  local pre = action.pre
  if type(pre) == "function" then
    local ok, yes = pcall(pre, state)
    return ok and yes and true or false
  end
  return satisfied(state, pre or {})
end

local function cost_of(action, state)
  local c = action.cost or 1
  if type(c) == "function" then
    local ok, value = pcall(c, state)
    c = ok and tonumber(value) or nil
  end
  return c and math.max(c, 0.1) or nil
end

-- A* over facts: from `facts` to a state that has the goal's `want`, cheapest chain first, at
-- most `max_steps` actions. The heuristic (the cheapest action while unsatisfied) never
-- overestimates. Returns the actions in order, or nil.
local function plan(goal, facts, usable)
  if satisfied(facts, goal.want) then return {} end
  local keys, seen_key = {}, {}
  local function add(k) if not seen_key[k] then seen_key[k] = true keys[#keys + 1] = k end end
  for k in pairs(goal.want) do add(k) end
  local cheapest = math.huge
  for _, a in ipairs(usable) do
    for k in pairs(a.effects) do add(k) end
    local c = type(a.cost) == "number" and a.cost or 0.1
    cheapest = math.min(cheapest, math.max(c, 0.1))
  end
  table.sort(keys)
  local function key(state)
    local parts = {}
    for i, k in ipairs(keys) do parts[i] = tostring(state[k]) end
    return table.concat(parts, "|")
  end
  local open, best = { { state = facts, path = {}, g = 0, f = cheapest } }, { [key(facts)] = 0 }
  local expanded = 0
  while #open > 0 and expanded < 500 do
    local at = 1
    for i = 2, #open do if open[i].f < open[at].f then at = i end end
    local node = table.remove(open, at)
    if satisfied(node.state, goal.want) then return node.path, node.g end
    expanded = expanded + 1
    if #node.path < G.max_steps then
      for _, a in ipairs(usable) do
        local c = applicable(a, node.state) and cost_of(a, node.state)
        if c then
          local next_state = setmetatable({}, { __index = node.state })
          for k, v in pairs(a.effects) do next_state[k] = v end
          local k2, g2 = key(next_state), node.g + c
          if k2 ~= key(node.state) and (best[k2] == nil or g2 < best[k2]) then
            best[k2] = g2
            local path = { unpack(node.path) }
            path[#path + 1] = a
            open[#open + 1] = { state = next_state, path = path, g = g2,
              f = g2 + (satisfied(next_state, goal.want) and 0 or cheapest) }
          end
        end
      end
    end
  end
  return nil
end

-- With `include_cooling`, an action backing off after a quick failure is still left out: one
-- that fails as it starts would otherwise be restarted at once, over and over.
local function usable_actions(include_cooling)
  local out = {}
  for name, a in pairs(G.actions) do
    local f = G.cooling(name) and G.failures[name]
    if not f or (include_cooling and not f.quick) then out[#out + 1] = a end
  end
  table.sort(out, function(a, b) return a.name < b.name end)
  return out
end

local function all_actions() return usable_actions(true) end

-- What the fast tier may choose (Agent.available): the top goal's next step, and every other
-- action that can start now and still leads to that goal, with what each would cost. Nothing
-- while the runner is off or holding.
local function offer(facts, usable)
  offered = {}
  if not G.on or G.held then return end
  local top
  for _, goal in ipairs(ordered()) do
    if goal.status == "active" and goal.plan[1] then
      top = goal
      break
    end
  end
  if not top then return end
  local first = top.plan[1]
  offered["goal:" .. first.name] = { goal = top.name,
    text = string.format("Next step toward goal %s (%s): %s", top.name, top.why or "", first.why or first.name) }
  for _, a in ipairs(usable) do
    local c = a ~= first and applicable(a, facts) and cost_of(a, facts)
    if c then
      local after = setmetatable({}, { __index = facts })
      for k, v in pairs(a.effects) do after[k] = v end
      local rest = plan(top, after, usable)
      if rest then
        local total = c
        for _, b in ipairs(rest) do total = total + (cost_of(b, after) or 0) end
        offered["goal:" .. a.name] = { goal = top.name,
          text = string.format("Another way toward goal %s (cost %d, the planned way less): %s", top.name,
            math.floor(total + 0.5), a.why or a.name) }
      end
    end
  end
end

-- Check every goal's `done`, plan each one still open, and signal what changed.
local function replan()
  dirty, planned_at = false, now()
  local facts = G.facts()
  local usable = usable_actions()
  for _, goal in ipairs(ordered()) do
    if goal.status ~= "done" and goal.status ~= "failed" then
      local finished, ok = false, nil
      if goal.done and not goal.standing then
        local fine, f, s = pcall(goal.done)
        if fine then
          finished, ok = f, s
        else
          goal.note = "done check failed: " .. tostring(f)
        end
      end
      if finished then
        goal.status, goal.plan = ok == false and "failed" or "done", {}
        goal.note = goal.status
        note("goal " .. goal.name .. " " .. goal.status)
        signal("GOAL_DONE", goal.name, ok ~= false)
      else
        local path = plan(goal, facts, usable)
        -- The standing goal never idles: if only backed-off actions lead on, try them again now.
        if not path and goal.standing then path = plan(goal, facts, all_actions()) end
        goal.plan = path or {}
        if path then
          goal.status, goal.note = "active", nil
        else
          local why = "no plan reaches " .. goal_text(goal) .. " now"
          if goal.status ~= "blocked" then
            note("goal " .. goal.name .. " blocked: " .. why)
            signal("GOAL_BLOCKED", goal.name, why)
          end
          goal.status, goal.note = "blocked", why
        end
      end
    end
  end
  offer(facts, usable)
  return facts
end

function G.list()
  replan()
  local out = {}
  for _, goal in ipairs(ordered()) do
    local names = {}
    for i, a in ipairs(goal.plan) do names[i] = a.name end
    out[#out + 1] = { name = goal.name, why = goal.why, priority = goal.priority, standing = goal.standing,
      status = goal.status, plan = names, note = goal.note }
  end
  return out
end

---------------------------------------------------------------------------------------------------
-- The runner

-- Ends that are not the action's fault: the backoff is for actions that fail on their own.
local INTERRUPTIONS = { "^replaced by", "^died", "^hold", "^attacked", "^hurt", "^fled", "^walk interrupted" }

local function interrupted(why)
  for _, p in ipairs(INTERRUPTIONS) do
    if tostring(why or ""):find(p) then return true end
  end
  return false
end

local function ended(ok, why)
  local c = G.current
  G.current, dirty = nil, true
  local key = c.action.name .. (c.args.subject and (":" .. tostring(c.args.subject)) or "")
  if ok then
    G.failures[key], G.failures[c.action.name] = nil, nil
    note("did " .. c.action.name .. " for " .. c.goal)
    -- Not for an instant action, which would then run every frame while its plan stands.
    kick = c.action.done ~= nil
    return
  end
  -- Whatever it left going (a fight, a walk) ends with it.
  if c.action.stop then pcall(c.action.stop, why) end
  if interrupted(why) then
    note(c.action.name .. " interrupted: " .. tostring(why))
  else
    local f = G.failures[key] or { n = 0 }
    f.n, f.why, f.quick = f.n + 1, why, now() - c.since < G.quick_fail
    local wait = math.min(G.retry_cap, G.retry_after * 2 ^ (f.n - 1))
    -- A place that would kill us is not tried again in twenty seconds.
    if tostring(why):find("^too dangerous") then wait = math.max(wait, G.danger_wait) end
    f.until_ = now() + wait
    f.waiting = true
    G.failures[key] = f
    note(c.action.name .. " failed: " .. tostring(why))
    signal("ACTION_FAILED", c.goal, c.action.name, tostring(why or "failed"))
  end
end

-- The agent's own signals: an activity's end (ours or another), a kill, a quest visit. Every one
-- is a reason to plan again.
if Agent and Agent.signal then
  local raw = Agent.raw_signal or Agent.signal
  Agent.raw_signal = raw
  local OURS = { GOAL_DONE = true, GOAL_BLOCKED = true, ACTION_FAILED = true, HOLD_EXPIRED = true }
  Agent.signal = function(name, ...)
    if not OURS[name] then dirty = true end
    if name == "QUESTS_DONE" then G.last_visit = { kind = select(1, ...), picked = select(2, ...), handed = select(3, ...) } end
    -- The activity's end goes out first, then what the runner makes of it (ACTION_FAILED).
    local result = raw(name, ...)
    local c = G.current
    if name == "INTENT_DONE" and c and select(1, ...) == c.intent then
      local _, ok, why = ...
      ended(ok, why)
    end
    return result
  end
end

-- Each action is an Agent intent `goal:<action>`, run by the runner or chosen by the fast tier
-- while offered; either way it works for the goal it was offered for.
register = function(action)
  local name = "goal:" .. action.name
  if not (Agent and Agent.intents) then return end
  Agent.intents[name] = {
    when = function() return offered[name] and offered[name].text or (action.why or action.name) end,
    -- Not while it backs off (a failure counts the same whoever started the action), and not to
    -- start while fighting or dead, as the runner starts nothing then; the running one stays shown.
    available = function()
      if not (G.on and not G.held and offered[name] ~= nil and not G.cooling(action.name)) then return false end
      local a = Agent.activity()
      if a and a.name == name then return true end
      return not UnitIsDeadOrGhost("player") and not UnitAffectingCombat("player") and not (Combat and Combat.active)
        and not (Combat and Combat.fleeing)
    end,
    run = function()
      local goal = G.goals[offered[name] and offered[name].goal or "progress"] or G.goals.progress
      local args = { goal = goal.name, data = goal.data, facts = last_facts }
      local z = current_zone()
      G.current = { goal = goal.name, action = action, intent = name, args = args, since = now(), zone = z and z.name }
      note("start " .. action.name .. " for " .. goal.name)
      local result = action.run(args)
      -- An action without `done` ends as `run` returns, unannounced by Agent.
      if not action.done then ended(true) end
      return result
    end,
    done = action.done,
    timeout = action.timeout,
  }
end

local function start(goal, action)
  local name = "goal:" .. action.name
  offered[name] = offered[name] or { goal = goal.name, text = action.why or action.name }
  offered[name].goal = goal.name
  Agent.run(name)
end

local function busy()
  if UnitIsDeadOrGhost("player") then return "dead" end
  -- A cinematic: nothing is in sight and nothing can walk, so an action started now would only
  -- fail and back off. The agent may skip it (StopCinematic); the runner never does.
  local held = AgentCinematic and AgentCinematic()
  if held then return "waiting: " .. held end
  if UnitAffectingCombat("player") or (Combat and Combat.active) then return "fighting" end
  -- Running from a fight is still the fight: no walk of the runner's takes the legs over.
  if Combat and Combat.fleeing then return "fleeing" end
  local a = Agent and Agent.activity and Agent.activity()
  if a then return "on " .. a.name end
  if Quests and Quests.busy and Quests.busy() then return "at a quest giver" end
end

function G.hold(spec)
  assert(type(spec) == "table" and type(spec.reason) == "string" and spec.reason ~= "", "a hold needs a reason")
  local seconds = tonumber(spec.seconds)
  assert(seconds and seconds > 0 and seconds < math.huge, "a hold needs a number of seconds")
  local t = now()
  local before = G.held or G.after
  local since = before and before.reason == spec.reason and before.since or t
  G.held, G.after = { reason = spec.reason, since = since, until_ = t + seconds }, nil
  local a = Agent and Agent.activity and Agent.activity()
  if G.current and a and a.name == G.current.intent then Agent.stop("hold: " .. spec.reason) end
  note("hold " .. math.floor(seconds) .. "s: " .. spec.reason)
  return true
end

function G.release()
  G.held, G.after, dirty = nil, nil, true
end

function G.holding()
  local h = G.held
  if not h then return nil end
  local t = now()
  return h.reason, math.floor(t - h.since), math.max(0, math.ceil(h.until_ - t))
end

local function check_hold(t)
  local h = G.held
  if h and t >= h.until_ then
    G.held = nil
    G.after = { reason = h.reason, since = h.since, until_ = t + G.grace }
    local waited = math.floor(t - h.since)
    note("hold over after " .. waited .. "s: " .. h.reason)
    signal("HOLD_EXPIRED", h.reason, waited)
  end
  if G.after and t >= G.after.until_ then G.after = nil end
end

---------------------------------------------------------------------------------------------------
-- Walks the planner starts by hand, through the control port's Lua (Agent.port_call brackets each
-- call): the runner steps aside for them, so the two never fight over where the character goes.

-- Inside a port call: the walk verbs as they were before it, and how deep we are in our own.
local port = { saved = nil, depth = 0 }

-- Where a walk goes, and how far that is: a place, a guid or a name in sight. nil when it is no
-- walk (an NPC within reach) or unknown.
local function walk_target(a, b, within)
  local pos = me()
  if type(a) == "number" and type(b) == "number" then
    return string.format("(%d, %d)", a, b), pos and far(pos, { x = a, y = b }) or 0
  end
  if type(a) == "table" and a.x then
    return string.format("(%d, %d)", a.x, a.y), pos and far(pos, a) or 0
  end
  if type(a) ~= "string" then return nil end
  for _, u in ipairs(seen(200)) do
    if u.guid == a or (u.name and u.name:lower() == a:lower()) then
      if within and u.distance <= within then return nil end
      return tostring(u.name or a), u.distance or 0
    end
  end
end

-- What the pre-empted action was doing: where it was going.
local function going(c)
  if c.args and c.args.going then return " (going to " .. tostring(c.args.going) .. ")" end
  if Move and Move.mode == "to" and Move.state then return " (" .. Move.state .. ")" end
  return ""
end

-- A walk, stop or sit the planner does by hand: hold the runner (ending the action it pre-empts,
-- whose stop would otherwise undo the walk) before it happens, and say so in the tool's output. A
-- walk's hold ends with the walk; a stop's or a sit's after `stop_hold` seconds.
local function planner_override(what, distance)
  if not G.on or (G.held and not G.held.by_hand) then return end
  local walk = what.walk
  local c = G.current
  local a = Agent and Agent.activity and Agent.activity()
  local pre_empted = c and a and a.name == c.intent and c
  local said = pre_empted and (c.action.name .. going(c)) or nil
  local seconds = walk and math.min(G.walk_hold_cap, math.floor(15 + 1.5 * (distance or 0) / 7)) or G.stop_hold
  G.hold({ reason = what.reason, seconds = seconds })
  G.held.by_hand = { since = now(), walk = walk, pre_empted = said }
  local text
  if walk then
    text = said
      and string.format("This walk pre-empted goal action %s for goal %s; the goal runner resumes when the walk ends (at most %d s).",
        said, c.goal, seconds)
      or string.format("The goal runner holds while this walk lasts, and resumes when it ends (at most %d s).", seconds)
  else
    text = string.format("%s%s; the goal runner holds for %d s, then resumes (Goals.release() resumes it now). The runner rests (eating from the bags), sells, loots, fights and walks by itself: shape it with goals, not by stopping it.",
      what.verb, said and (" pre-empted goal action " .. said .. " for goal " .. c.goal) or "", seconds)
  end
  note(what.reason .. (said and (" pre-empted " .. said) or ""))
  print(text)
end

local function returned(ok, ...)
  port.depth = port.depth - 1
  if not ok then error((...), 0) end
  return ...
end

-- What a walk verb's call does: a walk somewhere, or nothing (an NPC within `within` yards).
local function walking_to(within)
  return function(a, b)
    local where, distance = walk_target(a, b, within)
    if where then return { walk = true, reason = "your walk to " .. where }, distance end
  end
end

local function stopping(verb)
  return function() return { verb = verb, reason = "you " .. verb:lower() } end
end

-- A verb, wrapped: a call from the port's own code, outside an activity starting and outside
-- another wrapped verb, is the planner's own doing; `classify(args)` says what it does.
local function wrap(fn, classify)
  return function(a, b, ...)
    local mine = port.depth == 0 and not (Agent and Agent.starting)
    -- Deeper from here (the hold's own stop of the action, the verb's own calls) is not the planner's.
    port.depth = port.depth + 1
    if mine then
      local what, distance = classify(a, b)
      if what then pcall(planner_override, what, distance) end
    end
    return returned(pcall(fn, a, b, ...))
  end
end

local function sit_emote(token)
  return type(token) == "string" and token:upper() == "SIT" and { verb = "Sat down", reason = "you sat down" } or nil
end

-- The verbs wrapped for the length of one port call: { name, classify, on Move }.
local VERBS = {
  { "AgentMoveTo", walking_to() }, { "AgentInteract", walking_to(5) }, { "to", walking_to(), true },
  { "AgentStop", stopping("Stopped") }, { "SitStandOrDescendStart", stopping("Sat down") },
  { "DoEmote", sit_emote },
}

local function verb_home(v) return v[3] and Move or _G end

if Agent then
  Agent.port_call = function(begin)
    if begin and not port.saved then
      port.saved = {}
      for i, v in ipairs(VERBS) do
        local home = verb_home(v)
        local fn = home and home[v[1]]
        if type(fn) == "function" then
          local wrapped = wrap(fn, v[2])
          port.saved[i] = { fn = fn, wrapped = wrapped }
          home[v[1]] = wrapped
        end
      end
    elseif not begin and port.saved then
      -- Put back only what is still ours: a verb the call replaced itself stays replaced.
      for i, v in ipairs(VERBS) do
        local home, saved = verb_home(v), port.saved[i]
        if saved and home and home[v[1]] == saved.wrapped then home[v[1]] = saved.fn end
      end
      port.saved, port.depth = nil, 0
    end
  end
end

-- The planner's walk is over (or never started): the runner carries on at once.
local function check_walk_hold(t)
  local h = G.held
  if not (h and h.by_hand and h.by_hand.walk) or t - h.by_hand.since < 1 then return end
  local walk = AgentWalk and AgentWalk()
  if (walk and walk.status == "walking") or (Move and Move.mode == "to") then return end
  note("your walk is over: the runner carries on")
  G.release()
end

-- An action the planner taught that walks into a zone above us is stopped at its edge: travel
-- goes through a goal's data.zone and travel_to_zone, whose zone Goals.push checks. A built-in
-- action is not stopped (travel_to_zone's target was checked; a quest walk is the quest's).
local function check_zone(c)
  if not c or c.action.built_in then return end
  local a = Agent.activity()
  if not a or a.name ~= c.intent then return end
  local z = current_zone()
  if not z or z.name == c.zone or level() >= z.min - 1 then return end
  if Move and Move.free then Move.free() end
  if AgentStop then AgentStop() end
  local fits = {}
  for _, other in ipairs(G.zones()) do
    if other.fit == "ok" then fits[#fits + 1] = string.format("%s (%d-%d)", other.name, other.min, other.max) end
  end
  Agent.stop(string.format("walked into %s, levels %d-%d, above you at level %d: stopped there. Zones that fit: %s. Travel with a goal's data.zone (travel_to_zone), not an action of your own.",
    z.name, z.min, z.max, level(), #fits > 0 and table.concat(fits, ", ") or "none on this continent"))
end

local function tick(t)
  check_walk_hold(t)
  check_hold(t)
  check_zone(G.current)
  if not G.on then return end
  -- A backed-off action may be tried again now.
  for _, f in pairs(G.failures) do
    if f.waiting and t >= f.until_ then f.waiting, dirty = false, true end
  end
  if dirty or not planned_at or t - planned_at >= G.replan_every then replan() end
  if G.held then
    G.state = "holding: " .. G.held.reason
    return
  end
  if G.after then
    G.state = "hold over: " .. G.after.reason
    return
  end
  local why = busy()
  if why then
    G.state = why
    return
  end
  for _, goal in ipairs(ordered()) do
    if goal.status == "active" and goal.plan[1] then
      local action = goal.plan[1]
      -- Hurt or dry: rest before anything else, whatever the plan (a walk meets mobs too).
      local rest = G.actions.rest
      if rest and action ~= rest and (last_facts.hp_low or last_facts.mana_low) and not G.cooling("rest") then
        action = rest
      end
      G.state = goal.name .. ": " .. action.name
      return start(goal, action)
    end
  end
  G.state = "nothing achievable"
end

local frame = CreateFrame("Frame")
for _, event in ipairs({ "PLAYER_LEVEL_UP", "QUEST_LOG_UPDATE", "PLAYER_DEAD", "PLAYER_ALIVE", "PLAYER_UNGHOST",
  "PLAYER_REGEN_ENABLED", "ZONE_CHANGED_NEW_AREA", "PLAYER_ENTERING_WORLD", "MERCHANT_SHOW", "TRAINER_SHOW",
  "GOSSIP_SHOW" }) do
  frame:RegisterEvent(event)
end
-- What an NPC visit under way is there for ("trainer", "vendor"): a trainer or innkeeper who
-- greets with a gossip window first is asked for it.
local gossip_want
frame:SetScript("OnEvent", function(_, event)
  if event == "GOSSIP_SHOW" then
    if gossip_want and G.current and GetGossipOptions and SelectGossipOption then
      local options = { GetGossipOptions() }
      for i = 1, #options / 2 do
        if options[i * 2] == gossip_want then return SelectGossipOption(i) end
      end
    end
    return
  end
  dirty = true
  if event == "MERCHANT_SHOW" then G.merchant_at = now() end
end)
local since = 0
frame:SetScript("OnUpdate", function(_, dt)
  since = since + dt
  if since < 0.5 and not kick then return end
  since, kick = 0, false
  tick(now())
end)

---------------------------------------------------------------------------------------------------
-- Zones

function G.zones()
  local pos, lvl = me(), level()
  local faction = UnitFactionGroup and UnitFactionGroup("player")
  local here = GetRealZoneText and GetRealZoneText()
  local out = {}
  for _, z in ipairs(AgentZones and AgentZones() or {}) do
    if (not pos or z.map == pos.map) and not z.city and (z.faction == "contested" or z.faction == faction) then
      z.fit = lvl > z.max + 1 and "low" or (lvl < z.min - 1 and "high" or "ok")
      z.distance = pos and math.floor(far(pos, z)) or nil
      z.current = z.name == here
      out[#out + 1] = z
    end
  end
  table.sort(out, function(a, b) return (a.distance or 0) < (b.distance or 0) end)
  return out
end

-- Where travel_to_zone heads: the zone a goal names, else the nearest that fits.
local function travel_target(name)
  for _, z in ipairs(G.zones()) do
    if name then
      if z.name == name and not z.current then return z end
    elseif z.fit == "ok" and not z.current and not G.cooling("travel_to_zone:" .. z.name) then
      return z
    end
  end
end

local function top_zone_wish()
  for _, goal in ipairs(ordered()) do
    if goal.status == "active" and goal.data and goal.data.zone then return goal.data.zone end
  end
end

G.travel_target = travel_target

---------------------------------------------------------------------------------------------------
-- Built-in actions, made of the built-in modules

local function walking() return Move and Move.mode == "to" end

-- A walk's outcome once over: arrived, else why not.
local function walk_over()
  local _, arrived, outcome = Move.reached()
  return arrived, outcome
end

-- Why a walk that did not arrive ended the action: one someone else stopped short is an
-- interruption (the runner starts the action again), anything else the action's failure.
local function walk_failure(outcome, why)
  if outcome == "interrupted" then return "walk interrupted (stopped short by someone else) on the way: " .. why end
  return why
end

-- Talk to a remembered NPC: in sight, AgentInteract walks up; else walk to where they stood.
local function visit(st)
  local p = st.place
  for _, u in ipairs(seen(80)) do
    if u.guid == p.guid or (u.name == p.name and u.kind == "unit") then
      if not st.asked or now() - st.asked >= 8 then
        st.asked = now()
        AgentInteract(u.guid)
      end
      return false
    end
  end
  if not st.walked then
    st.walked = true
    local ok, why = Move.to(p.x, p.y, p.z)
    if not ok then return true, false, "no way to " .. tostring(p.name) .. ": " .. tostring(why) end
    return false
  end
  if walking() then return false end
  local arrived, outcome = walk_over()
  if not arrived then
    return true, false, walk_failure(outcome, "could not reach " .. tostring(p.name) .. " (" .. tostring(outcome) .. ")")
  end
  forget(p)
  return true, false, tostring(p.name) .. " is not where they stood"
end

-- For modules that add NPC visits of their own (wp.professions): the units in sight this pass, a
-- visit to a remembered NPC (finished, ok, why), and the gossip option a greeting is answered with
-- ("trainer", "vendor"; nil when the visit is over).
G.nearby, G.visit = nearby, visit
function G.ask_gossip(want) gossip_want = want end

local quest_visit = {}

-- Where a finished quest is handed in, if the client knows and it is on this continent.
local function hand_in_of(q)
  local w = q.complete and Quests.where and Quests.where(q.id)
  local h, pos = w and w.hand_in, me()
  if h and h.map and pos and pos.map and h.map ~= pos.map then return nil end
  return h
end

-- A hand-in that could not be made, said so the planner can act on it: which quest, where, how
-- far short the walk ended, and what the quest log says (the NPC and the town it names).
local function hand_in_failure(outcome)
  local t, pos = quest_visit.target, me()
  if not t then return "nobody to hand in to here" end
  local short = pos and math.floor(far(pos, t)) or 0
  local says = Quests.text and Quests.text(t.id)
  says = says and (' The quest log says: "' .. says .. '"') or ""
  if short > G.hand_in_reach then
    return string.format("the walk to the hand-in of %s at (%d, %d) ended %d yards short (%s): no route on foot from here, so go by another way (a waypoint through the town gate, a road).%s",
      t.title, t.x, t.y, short, tostring(outcome), says)
  end
  return string.format("nobody in sight takes %s at its hand-in (%d, %d): the point is the centre of a map area, so look within %d yards for the NPC or object the quest names.%s",
    t.title, t.x, t.y, G.hand_in_reach, says)
end

G.action({
  name = "hand_in_quests",
  stop = function() Move.free() end,
  why = "Hand in finished quests: to the giver in sight, else where the quest says.",
  pre = function(f)
    if f.hand_in_near then return true end
    if not f.quest_ready then return false end
    if pass.hand_in == nil then
      pass.hand_in = false
      for _, q in ipairs(quests()) do
        if hand_in_of(q) and not G.cooling("hand_in_quests:" .. q.title) then pass.hand_in = true end
      end
    end
    return pass.hand_in
  end,
  effects = { quest_ready = false, hand_in_near = false, progressed = true },
  -- In sight, the cheapest step there is. Else by the walk, as the NPC errands are but one less
  -- (2 + yards / 200, at most 6, against their 3 + ...), so a hand-in comes before training or
  -- selling at a like distance, as `progress` says (quests first): a flat 4 put any trainer
  -- within 200 yards first.
  cost = function(f)
    if f.hand_in_near then return 1 end
    local pos, best = me(), nil
    for _, q in ipairs(quests()) do
      local h = hand_in_of(q)
      if h and pos and not G.cooling("hand_in_quests:" .. q.title) then
        local d = far(pos, h)
        if not best or d < best then best = d end
      end
    end
    return 2 + (best and math.min(best / 200, 4) or 2)
  end,
  run = function(args)
    quest_visit = { talking = false, since = now() }
    G.last_visit = nil
    if marker("reward") then
      quest_visit.talking = true
      Quests.attempt("reward")
      return Quests.hand_in()
    end
    for _, q in ipairs(Quests.list()) do
      local h = hand_in_of(q)
      if h and not G.cooling("hand_in_quests:" .. q.title) then
        args.subject, args.going = q.title, "the hand-in of " .. q.title
        quest_visit.target = { id = q.id, title = q.title, x = h.x, y = h.y }
        local ok, why = Move.to(h.x, h.y)
        if not ok then error(hand_in_failure("no way: " .. tostring(why)), 0) end
        return
      end
    end
    error("no finished quest has a known hand-in")
  end,
  done = function()
    pass = {}
    if not quest_visit.talking then
      if marker("reward") then
        if walking() then Move.free() end
        quest_visit.talking = true
        Quests.attempt("reward")
        Quests.hand_in()
      elseif not walking() then
        local arrived, outcome = walk_over()
        return true, false, walk_failure(outcome, hand_in_failure(arrived and "the route stops there" or ("walk " .. tostring(outcome))))
      end
      return false
    end
    if Quests.busy() then return false end
    Quests.settle()
    local v = G.last_visit
    if v and (v.handed or 0) + (v.picked or 0) > 0 then return true, true end
    return true, false, "handed nothing in"
  end,
  timeout = 600,
})

G.action({
  name = "pick_up_quests",
  why = "Take the quests the givers in sight offer.",
  pre = { quest_giver_near = true },
  effects = { quest_giver_near = false, objectives_open = true, progressed = true },
  cost = 2,
  run = function()
    G.last_visit = nil
    Quests.attempt("available")
    Quests.pick_up()
  end,
  done = function()
    if Quests.busy() then return false end
    Quests.settle()
    local v = G.last_visit
    if v and (v.picked or 0) + (v.handed or 0) > 0 then return true, true end
    return true, false, "took nothing"
  end,
  timeout = 150,
})

-- The open objectives as { quest, index, text, kind, name, have, need, poi }.
local function objectives()
  local out, pos = {}, me()
  for _, q in ipairs(quests()) do
    if not q.complete then
      for i, o in ipairs(q.objectives) do
        if not o.done and o.text then
          local name, have, need = o.text:match("^(.-) slain: (%d+)/(%d+)")
          if not name then name, have, need = o.text:match("^(.-): (%d+)/(%d+)") end
          local w = Quests.where and Quests.where(q.id)
          local poi
          for _, p in ipairs(w and w.objectives or {}) do
            if p.index == i - 1 then poi = p end
          end
          if not poi and w then
            for _, p in ipairs(w.objectives) do
              if not poi or (pos and far(pos, p) < far(pos, poi)) then poi = p end
            end
          end
          out[#out + 1] = { quest = q, index = i, text = o.text, kind = o.kind, name = name or o.text,
            have = tonumber(have), key = q.id .. "." .. i, poi = poi }
        end
      end
    end
  end
  return out
end

-- What the objective wants dealt with, in sight: a mob or an object of its name, or (an item
-- with no object of its name) a mob that fits near its area. A mob that is a safe pull now
-- (G.safe) comes before one that is not; the second return says why the one found is not.
local function objective_target(o)
  local lower = o.name:lower()
  local list = seen(100)
  local best
  for _, u in ipairs(list) do
    if u.distance <= 60 then
      local named = u.name and u.name:lower() == lower
      local fight = u.kind == "unit" and not u.dead and u.reaction ~= "friendly" and not (u.tapped and not u.tapped_by_me)
      local ok = (named and (fight or u.kind == "gameobject"))
        or (o.kind == "item" and fight and G.fits(u) and (not o.poi or far(u, o.poi) < 60))
      if ok then
        local safe, why = true, nil
        if fight and u.kind == "unit" then safe, why = G.safe(u, list) end
        local c = { u = u, named = named, safe = safe, why = why }
        local better = not best
        if best and c.safe ~= best.safe then
          better = c.safe
        elseif best and c.named ~= best.named then
          better = c.named
        elseif best then
          better = u.distance < best.u.distance
        end
        if better then best = c end
      end
    end
  end
  if not best then return nil end
  return best.u, (not best.safe) and (best.why or "not a safe fight") or nil
end

-- Whether the hostiles in sight make this a place to stay out of: three or more, most of them
-- above the levels we pull. Returns the reason, or nil.
local function danger()
  if not Survive then return nil end
  local _, high = Survive.band()
  local n, above, low, top = 0, 0, nil, nil
  for _, u in ipairs(seen(70)) do
    if u.kind == "unit" and not u.dead and u.reaction == "hostile" and u.level and not u.critter then
      n = n + 1
      if u.level > high then above = above + 1 end
      low, top = math.min(low or u.level, u.level), math.max(top or u.level, u.level)
    end
  end
  if n >= 3 and above * 2 > n then
    return string.format("%d hostile mobs of level %d-%d here, and we pull to level %d", n, low, top, high)
  end
end

-- The objective to work: one with something to do in sight, else the nearest with an area.
local function pick_objective()
  local pos, best, best_d = me(), nil, math.huge
  for _, o in ipairs(objectives()) do
    if not G.cooling("work_objective:" .. o.key) then
      if objective_target(o) then return o end
      local d = (o.poi and pos) and far(pos, o.poi) or nil
      if d and d < best_d then best, best_d = o, d end
    end
  end
  return best
end

local job = {}

G.action({
  name = "work_objective",
  stop = function()
    Combat.stop()
    Move.free()
  end,
  why = "Work a quest objective: go to its area, fight the mob or use the object it names.",
  pre = function(f)
    if f.hp_low or f.mana_low or f.gear_broken or not f.objectives_open then return false end
    -- Once per planning pass: the search asks for every state it visits.
    if pass.objective == nil then pass.objective = pick_objective() or false end
    return pass.objective ~= false
  end,
  effects = { progressed = true },
  cost = function(f) return f.bags_full and 7 or 5 end,
  run = function(args)
    local o = pick_objective()
    assert(o, "no objective to work")
    args.subject, args.going = o.key, "the area for " .. o.text
    job = { o = o, since = now(), uses = 0 }
    note("objective: " .. o.text .. " (" .. o.quest.title .. ")")
  end,
  done = function()
    pass = {}
    local o = job.o
    for _, q in ipairs(quests()) do
      if q.id == o.quest.id then
        if q.complete then return true, true end
        local now_o = q.objectives[o.index]
        if not now_o or now_o.done or now_o.text ~= o.text then return true, true end
      end
    end
    if Combat.active then return false end
    if job.fighting then
      local _, _, outcome = Combat.engaged(job.fighting)
      job.fighting = nil
      if outcome == "died" then return true, false, "died fighting" end
      if outcome == "fled" then return true, false, "fled a fight" end
    end
    local t, unsafe = objective_target(o)
    if t and unsafe then
      -- Our own state first: hurt (a fight on the way) means rest, not giving the objective up.
      local ready, why = true, nil
      if Survive then ready, why = Survive.ready() end
      if not ready then return true, false, "hurt: " .. tostring(why) end
      -- The mob itself: stand off a little while (a patrol moves on, a pack spreads), then leave
      -- the objective for later rather than die on it.
      if walking() then Move.free() end
      job.unsafe_since = job.unsafe_since or now()
      if now() - job.unsafe_since < G.unsafe_wait then return false end
      return true, false, string.format("%s is not a safe fight now: %s", tostring(t.name), unsafe)
    end
    job.unsafe_since = nil
    -- Nothing to fight for it, and what is in sight would kill us: out, and not back for a while.
    if not t or t.kind == "gameobject" then
      local risk = danger()
      if risk then return true, false, "too dangerous for now: " .. risk end
    end
    if t then
      if walking() then Move.free() end
      if t.kind == "gameobject" then
        if job.used_at and now() - job.used_at < 5 then return false end
        if job.uses >= 3 then return true, false, "using " .. tostring(t.name) .. " did nothing" end
        job.uses, job.used_at = job.uses + 1, now()
        AgentInteract(t.guid)
      elseif Combat.engage(t.guid) then
        job.fighting = t.guid
      end
      return false
    end
    local pos = me()
    if not o.poi or not pos then return true, false, "nothing in sight for " .. o.text .. " and no area to go to" end
    if far(pos, o.poi) > 25 then
      if walking() then return false end
      if job.walked then
        local arrived, outcome = walk_over()
        if not arrived then
          return true, false, walk_failure(outcome, "could not reach the area for " .. o.text .. " (" .. tostring(outcome) .. ")")
        end
      end
      job.walked = true
      local ok, why = Move.to(o.poi.x, o.poi.y)
      if not ok then return true, false, "no way to the area for " .. o.text .. ": " .. tostring(why) end
      return false
    end
    -- In the area and nothing in sight: look around it a little, then give up for now.
    if walking() then return false end
    job.looks = (job.looks or 0) + 1
    if job.looks > 3 then return true, false, "nothing for " .. o.text .. " around its area" end
    local a = math.random() * 2 * math.pi
    Move.to(o.poi.x + math.cos(a) * 20, o.poi.y + math.sin(a) * 20)
    return false
  end,
  timeout = 600,
})

-- An action that talks to the nearest remembered NPC of a kind (`kind(facts)` picks it), with
-- `subject(facts)` naming what a failure backs off, until `finished(st)` says it is over.
local function npc_action(spec)
  local st = {}
  G.action({
    name = spec.name, why = spec.why, pre = spec.pre, effects = spec.effects,
    stop = function()
      gossip_want = nil
      Move.free()
    end,
    -- Further away costs more, but never so much that grinding beats it for good.
    cost = function(f)
      local p, pos = place(spec.kind(f)), me()
      return 3 + ((p and pos) and math.min(far(pos, p) / 200, 4) or 0)
    end,
    run = function(args)
      local kind = spec.kind(args.facts or {})
      st = { since = now(), place = place(kind) }
      assert(st.place, "no " .. kind .. " known")
      args.subject = spec.subject and spec.subject(args.facts or {}) or nil
      args.going = st.place.name
      gossip_want = kind == "trainer" and "trainer" or "vendor"
    end,
    done = function()
      local over, ok, reason = spec.finished(st)
      if not over then over, ok, reason = visit(st) end
      if over then gossip_want = nil end
      return over, ok, reason
    end,
    timeout = 900,
  })
end

npc_action({ name = "train", why = "Learn the new ranks from our class trainer.",
  kind = function() return "trainer" end,
  pre = { can_train = true, trainer_known = true }, effects = { can_train = false, progressed = true },
  finished = function() return Upkeep.trained_level == level(), true end })

-- Full bags send us to the nearest merchant of any kind (every one buys any item); worn-out gear to
-- the nearest who repairs. Bags a visit could not empty back the bag errand off ("bags"), not the
-- repairs; and with nothing a merchant would take, there is no errand at all.
npc_action({ name = "sell_and_repair", why = "Sell the junk and repair at a merchant.",
  kind = function(f) return (f.needs_repair and f.repair_known) and "repair" or "vendor" end,
  subject = function(f) return f.bags_full and "bags" or nil end,
  pre = function(f)
    if f.needs_repair and f.repair_known then return true end
    if not (f.bags_full and f.vendor_known) or G.cooling("sell_and_repair:bags") then return false end
    if pass.sellable == nil then pass.sellable = not (Upkeep and Upkeep.sellable) or Upkeep.sellable() end
    return pass.sellable
  end,
  effects = { bags_full = false, needs_repair = false, gear_broken = false, progressed = true },
  finished = function(st)
    if not (G.merchant_at and G.merchant_at >= st.since) then return false end
    if now() - G.merchant_at < 1.5 then return false end
    if CloseMerchant then CloseMerchant() end
    local facts = G.facts()
    if facts.bags_full then
      return true, false, "the bags are still full after selling: they hold "
        .. (Upkeep and Upkeep.filling and Upkeep.filling() or "?")
        .. ". Free slots yourself (sell or destroy what you can spare, or let Upkeep.sell_when_full sell a type), or get a bag; selling waits before it is tried again"
    end
    return true, true
  end })

-- Items a full bag never sells at a merchant, by name.
function G.keep(names)
  assert(type(names) == "table", "Goals.keep takes a list of item names")
  for _, name in ipairs(names) do Upkeep.keep[tostring(name)] = true end
  local out = {}
  for name in pairs(Upkeep.keep) do out[#out + 1] = name end
  table.sort(out)
  return out
end

npc_action({ name = "buy_food", why = "Buy food (and drink) where it is sold: eating between fights is how health comes back.",
  kind = function() return "food" end,
  pre = { needs_food = true, food_vendor_known = true }, effects = { needs_food = false, progressed = true },
  finished = function(st)
    if not (G.merchant_at and G.merchant_at >= st.since) then return false end
    if now() - G.merchant_at < 1.5 then return false end
    if CloseMerchant then CloseMerchant() end
    if G.facts().needs_food then return true, false, tostring(st.place.name) .. " sold nothing we can eat and afford" end
    return true, true
  end })

-- A living hostile close enough to notice us where we stand (its aggro radius and a margin).
local function hostile_close()
  for _, u in ipairs(seen(60)) do
    if u.kind == "unit" and not u.dead and u.reaction == "hostile" and not u.critter
      and u.distance < (Survive and Survive.aggro(u.level) or 20) + 8 then
      return u
    end
  end
end

local rest = {}

-- Rest until health and mana are full; then AGENT_RESTED(health, mana) says so, for whoever
-- waits on it (the planner, instead of a timer).
G.action({
  name = "rest",
  stop = function()
    Move.free()
    if Survive then Survive.stand() end
  end,
  why = "Step clear of hostiles, then eat, drink, bandage or sit until health and mana are back.",
  pre = function(f) return (f.hp_low or f.mana_low) and not f.in_combat end,
  effects = { hp_low = false, mana_low = false, pull_ok = true },
  cost = 2,
  run = function()
    rest = { since = now() }
    -- Not beside a mob that will wander over: move off first.
    local p = hostile_close() and Survive and Survive.escape_point(nil)
    if p and Move.to(p.x, p.y) then rest.moving = true end
  end,
  done = function()
    if UnitAffectingCombat("player") then return true, false, "attacked while resting" end
    if rest.moving then
      if walking() then return false end
      rest.moving = nil
    end
    local hp = ratio(UnitHealth("player"), UnitHealthMax("player"))
    local mana = (UnitPowerType and UnitPowerType("player") ~= 0) and 1 or ratio(UnitPower("player", 0), UnitPowerMax("player", 0))
    if hp >= 1 and mana >= 1 then
      if Survive then Survive.stand() end
      signal("RESTED", UnitHealth("player"), UnitPowerType and UnitPowerType("player") == 0 and UnitPower("player", 0) or nil)
      return true, true
    end
    if not rest.ate then
      rest.ate = now()
      rest.did = Survive and Survive.eat() or ""
      note("rest: " .. tostring(rest.did))
    elseif Survive and now() - rest.ate >= 25 and rest.did ~= "sat down" then
      -- The food ran out before we were whole: the next one.
      rest.ate, rest.did = now(), Survive.eat()
    end
    return false
  end,
  timeout = 240,
})

local grind = {}

G.action({
  name = "grind",
  why = "Fight the nearest mob in sight that gives XP and is a safe pull (alone, in our level band).",
  pre = function(f) return f.mobs_for_xp and not f.hp_low and not f.mana_low and not f.gear_broken end,
  effects = { progressed = true },
  cost = function(f) return f.bags_full and 12 or 8 end,
  run = function()
    local list = seen(100)
    local best, why
    for _, u in ipairs(list) do
      if G.fits(u) then
        local ok, reason = G.safe(u, list)
        if ok and (not best or u.distance < best.distance) then best = u end
        why = why or reason
      end
    end
    assert(best, "no safe pull in sight" .. (why and (": " .. why) or ""))
    local ok, refused = Combat.engage(best.guid)
    assert(ok, "could not engage " .. tostring(best.name) .. ": " .. tostring(refused))
    grind.guid = best.guid
  end,
  stop = function() Combat.stop() end,
  done = function()
    local over, won, outcome = Combat.engaged(grind.guid)
    if not over then return false end
    return true, won, outcome
  end,
  timeout = 180,
})

local seek = {}

G.action({
  name = "seek_mobs",
  stop = function() Move.free() end,
  why = "Walk on to where mobs that give XP were seen, else further afield in this zone.",
  pre = function(f) return not f.mobs_for_xp and f.zone_fit == "ok" end,
  effects = { mobs_for_xp = true },
  cost = 6,
  run = function()
    local pos = me()
    assert(pos, "not in the world")
    seek = { since = now() }
    table.sort(sightings, function(a, b) return a.at < b.at end)
    for i = #sightings, 1, -1 do
      local s = sightings[i]
      if far(pos, s) > 60 then
        local ok = Move.to(s.x, s.y)
        if ok then return end
      end
    end
    for _ = 1, 6 do
      local a = math.random() * 2 * math.pi
      if Move.to(pos.x + math.cos(a) * 100, pos.y + math.sin(a) * 100) then return end
    end
    error("no way on from here")
  end,
  done = function()
    pass = {}
    for _, u in ipairs(nearby()) do
      if G.safe(u, nearby()) then
        if walking() then Move.free() end
        return true, true
      end
    end
    if walking() then return false end
    local _, outcome = walk_over()
    return true, false, walk_failure(outcome, "no mobs that give XP and are safe to pull on the way")
  end,
  timeout = 120,
})

local travel = {}

G.action({
  name = "travel_to_zone",
  stop = function() Move.free() end,
  why = "Travel to a zone that suits our level (or the one a goal names).",
  pre = function(f)
    -- A zone that suits us is not left because nothing safe is in sight just now (a death has
    -- lowered what we pull, a camp is too crowded): that is seek_mobs' to walk off. Leaving is
    -- for a zone outgrown or too high, or the planner's choice (a goal's data.zone).
    if f.zone_fit == "ok" and not top_zone_wish() then return false end
    if pass.travel == nil then pass.travel = travel_target(top_zone_wish()) or false end
    return pass.travel ~= false
  end,
  effects = { zone_fit = "ok", mobs_for_xp = true },
  cost = function(f) return f.zone_fit == "ok" and 40 or 20 end,
  run = function(args)
    local z = travel_target(args.data and args.data.zone or top_zone_wish())
    assert(z, "no zone to travel to")
    args.subject, args.going = z.name, z.name
    travel = { zone = z }
    note("travel to " .. z.name .. " (levels " .. z.min .. "-" .. z.max .. ")")
    local ok, why = Move.to(z.x, z.y)
    assert(ok, why)
  end,
  done = function()
    if GetRealZoneText and GetRealZoneText() == travel.zone.name then
      if walking() then Move.free() end
      return true, true
    end
    if walking() then return false end
    local arrived, outcome = walk_over()
    if arrived then return true, true end
    return true, false, walk_failure(outcome, "could not reach " .. travel.zone.name .. " (" .. tostring(outcome) .. ")")
  end,
  timeout = 1800,
})

-- Everything defined so far is built in; an action the planner teaches (or redefines) is not.
for _, a in pairs(G.actions) do a.built_in = true end

G.push({ name = "progress", standing = true, priority = 0, want = { progressed = true },
  why = "Keep levelling: quests first, then training and upkeep, else grind, else find a better place." })

return G
