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
--   one): quest_ready, objectives_open, quest_giver_near, hand_in_near, bags_full, needs_repair,
--   can_train, trainer_known, vendor_known, repair_known (seen this session, and where:
--   Goals.places), hp_low,
--   mana_low, mobs_for_xp (living hostiles in sight that give XP), zone_fit ("low": the zone is
--   below us, "ok", "high": above us), in_combat, dead.
-- Actions:
--   Goals.action{ name, why, pre = { fact = value } | fn(facts), effects = { fact = value },
--                 cost = n | fn(facts), run = fn(args), done = fn() → finished, ok, why, timeout,
--                 stop = fn(why) }
--       `args` is { goal, data, facts }; `run` may set args.subject (the quest objective, the zone)
--       so a failure backs off that one, not the whole action; `stop` undoes what an action that
--       ended badly left going (its fight, its walk). Built in: hand_in_quests,
--       pick_up_quests, work_objective, train, sell_and_repair, rest, grind, seek_mobs,
--       travel_to_zone. A plan is the cheapest chain of actions (A* over the facts) from now to
--       the goal's `want`; plans are made again on the agent's signals (an activity's end, a
--       kill, a quest visit) and the client's (level up, quest log, death, zone), else every
--       `replan_every` seconds.
-- Runner (`Goals.on`, off until the agent turns it on): with no Agent activity running, no hold,
--   not fighting or dead, the next action of the top goal that has a plan runs AS an Agent
--   activity named `goal:<action>`, so the fast tier and the status line see it. A failed action
--   is AGENT_ACTION_FAILED(goal, action, why) and waits `retry_after` seconds before it is tried
--   again, doubling on each failure in a row (an interruption, a death or a hold is no failure).
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
-- `Goals.state` says what the runner is doing; `Goals.log` keeps its last few lines.

local G = {
  on = false, goals = {}, actions = {}, places = {}, log = {}, state = nil,
  replan_every = 5, retry_after = 20, retry_cap = 300, grace = 3, max_steps = 5,
  -- Health below this out of combat is hp_low; mana below `mana_below` is mana_low.
  rest_below = 0.5, mana_below = 0.35,
  -- Mobs this many levels above us still count for XP.
  above = 2,
  -- A walk that ends within this many yards of a quest's hand-in point has arrived at it.
  hand_in_reach = 60,
  failures = {}, current = nil, held = nil, after = nil,
}

local facts_fns, fact_order, fact_errors = {}, {}, {}
-- What the fast tier is offered now (`goal:<action>` intent -> { goal, text }), and the intent
-- each action runs as (set up with the runner, below).
local offered, register = {}, nil
local dirty, planned_at, last_facts = true, nil, {}

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
-- service, and not grey nor too far above us.
function G.fits(u)
  local lvl = level()
  return u.kind == "unit" and not u.dead and (u.reaction == "hostile" or u.reaction == "neutral")
    and not (u.tapped and not u.tapped_by_me) and not u.npc and u.level ~= nil
    and u.level > grey(lvl) and u.level <= lvl + G.above
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

-- Remember where the NPCs we need stood, as a player remembers the trainer's house.
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
          G.places.trainer = place
        elseif s == "repair" then
          G.places.repair = place
        elseif s == "vendor" and not (G.places.repair and G.places.repair.guid == u.guid) then
          G.places.vendor = place
        end
      end
    end
  end
end

-- A remembered place on this continent, else nil.
local function place(kind)
  local p, pos = G.places[kind], me()
  if p and pos and p.map and pos.map ~= p.map then return nil end
  return p
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
    if u.kind == "unit" and not u.dead and u.level and (u.reaction == "hostile" or u.reaction == "neutral")
      and not u.npc then
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
  { "hp_low", function() return ratio(UnitHealth("player"), UnitHealthMax("player")) < G.rest_below end },
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
    return free ~= nil and free <= 1
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
  { "vendor_known", function() return (place("repair") or place("vendor")) ~= nil end },
  -- Repairs need a repair NPC: any vendor buys junk, but only some fix gear.
  { "repair_known", function() return place("repair") ~= nil end },
  { "mobs_for_xp", function()
    local any = false
    for _, u in ipairs(nearby()) do
      if G.fits(u) then
        any = true
        G.sighted(u)
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

local function usable_actions(include_cooling)
  local out = {}
  for name, a in pairs(G.actions) do
    if include_cooling or not G.cooling(name) then out[#out + 1] = a end
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
local INTERRUPTIONS = { "^replaced by", "^died", "^hold", "^attacked" }

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
    return
  end
  -- Whatever it left going (a fight, a walk) ends with it.
  if c.action.stop then pcall(c.action.stop, why) end
  if interrupted(why) then
    note(c.action.name .. " interrupted: " .. tostring(why))
  else
    local f = G.failures[key] or { n = 0 }
    f.n, f.why = f.n + 1, why
    f.until_ = now() + math.min(G.retry_cap, G.retry_after * 2 ^ (f.n - 1))
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
    available = function() return G.on and not G.held and offered[name] ~= nil end,
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
  if UnitAffectingCombat("player") or (Combat and Combat.active) then return "fighting" end
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
      G.state = goal.name .. ": " .. goal.plan[1].name
      return start(goal, goal.plan[1])
    end
  end
  G.state = "nothing achievable"
end

local frame = CreateFrame("Frame")
for _, event in ipairs({ "PLAYER_LEVEL_UP", "QUEST_LOG_UPDATE", "PLAYER_DEAD", "PLAYER_ALIVE", "PLAYER_UNGHOST",
  "PLAYER_REGEN_ENABLED", "ZONE_CHANGED_NEW_AREA", "PLAYER_ENTERING_WORLD", "MERCHANT_SHOW", "TRAINER_SHOW" }) do
  frame:RegisterEvent(event)
end
frame:SetScript("OnEvent", function(_, event)
  dirty = true
  if event == "MERCHANT_SHOW" then G.merchant_at = now() end
end)
local since = 0
frame:SetScript("OnUpdate", function(_, dt)
  since = since + dt
  if since < 0.5 then return end
  since = 0
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

-- Talk to a remembered NPC: in sight, AgentInteract walks up; else walk to where they stood.
local function visit(kind, st)
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
  if not arrived then return true, false, "could not reach " .. tostring(p.name) .. " (" .. tostring(outcome) .. ")" end
  G.places[kind] = nil
  return true, false, tostring(p.name) .. " is not where they stood"
end

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
  cost = function(f) return f.hand_in_near and 1 or 4 end,
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
        args.subject = q.title
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
        return true, false, hand_in_failure(arrived and "the route stops there" or ("walk " .. tostring(outcome)))
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
-- with no object of its name) a mob that fits near its area.
local function objective_target(o)
  local lower = o.name:lower()
  local best
  for _, u in ipairs(seen(60)) do
    local named = u.name and u.name:lower() == lower
    local fight = u.kind == "unit" and not u.dead and u.reaction ~= "friendly" and not (u.tapped and not u.tapped_by_me)
    local ok = (named and (fight or u.kind == "gameobject"))
      or (o.kind == "item" and fight and G.fits(u) and (not o.poi or far(u, o.poi) < 60))
    if ok and (not best or (named and not best.named) or (named == best.named and u.distance < best.u.distance)) then
      best = { u = u, named = named }
    end
  end
  return best and best.u
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
    if f.hp_low or f.mana_low or not f.objectives_open then return false end
    -- Once per planning pass: the search asks for every state it visits.
    if pass.objective == nil then pass.objective = pick_objective() or false end
    return pass.objective ~= false
  end,
  effects = { progressed = true },
  cost = function(f) return f.bags_full and 7 or 5 end,
  run = function(args)
    local o = pick_objective()
    assert(o, "no objective to work")
    args.subject = o.key
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
    end
    local t = objective_target(o)
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
        if not arrived then return true, false, "could not reach the area for " .. o.text .. " (" .. tostring(outcome) .. ")" end
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

local function npc_action(name, kind, why, pre, effects, finished)
  local st = {}
  G.action({
    name = name, why = why, pre = pre, effects = effects,
    stop = function() Move.free() end,
    -- Further away costs more, but never so much that grinding beats it for good.
    cost = function()
      local p, pos = place(kind) or (kind == "repair" and place("vendor")), me()
      return 3 + ((p and pos) and math.min(far(pos, p) / 200, 4) or 0)
    end,
    run = function()
      st = { since = now(), place = place(kind) or (kind == "repair" and place("vendor")) }
      assert(st.place, "no " .. kind .. " known")
    end,
    done = function()
      local over, ok, reason = finished(st)
      if over then return over, ok, reason end
      return visit(st.place == G.places.vendor and "vendor" or kind, st)
    end,
    timeout = 900,
  })
end

npc_action("train", "trainer", "Learn the new ranks from our class trainer.",
  { can_train = true, trainer_known = true }, { can_train = false, progressed = true },
  function() return Upkeep.trained_level == level(), true end)

npc_action("sell_and_repair", "repair", "Sell the junk and repair at a merchant.",
  function(f) return (f.bags_full and f.vendor_known) or (f.needs_repair and f.repair_known) end,
  { bags_full = false, needs_repair = false, progressed = true },
  function(st)
    if not (G.merchant_at and G.merchant_at >= st.since) then return false end
    if now() - G.merchant_at < 1.5 then return false end
    if CloseMerchant then CloseMerchant() end
    local facts = G.facts()
    if facts.bags_full then return true, false, "the bags are still full after selling the junk" end
    return true, true
  end)

-- What to eat or drink from the bags: the item whose use is Food (health) or Drink (mana).
local function provision(want)
  for bag = 0, 4 do
    for slot = 1, (GetContainerNumSlots(bag) or 0) do
      local link = GetContainerItemLink(bag, slot)
      local spell = link and GetItemSpell and GetItemSpell(link)
      if spell == want or spell == "Refreshment" then return bag, slot end
    end
  end
end

local rest = {}

G.action({
  name = "rest",
  why = "Eat, drink or sit until health and mana are back.",
  pre = function(f) return (f.hp_low or f.mana_low) and not f.in_combat end,
  effects = { hp_low = false, mana_low = false },
  cost = 2,
  run = function()
    rest = { since = now() }
    local used = false
    for _, need in ipairs({ { "hp_low", "Food" }, { "mana_low", "Drink" } }) do
      -- Not `cond and provision(...)`: `and` keeps only the first of provision's two returns.
      local bag, slot
      if last_facts[need[1]] then bag, slot = provision(need[2]) end
      if bag then
        UseContainerItem(bag, slot)
        used = true
      end
    end
    if not used and SitStandOrDescendStart then SitStandOrDescendStart() end
  end,
  done = function()
    if UnitAffectingCombat("player") then return true, false, "attacked while resting" end
    local hp = ratio(UnitHealth("player"), UnitHealthMax("player"))
    local mana = (UnitPowerType and UnitPowerType("player") ~= 0) and 1 or ratio(UnitPower("player", 0), UnitPowerMax("player", 0))
    if hp >= 0.95 and mana >= 0.95 then return true, true end
    return false
  end,
  timeout = 90,
})

local grind = {}

G.action({
  name = "grind",
  why = "Fight the nearest mob in sight that gives XP.",
  pre = function(f) return f.mobs_for_xp and not f.hp_low and not f.mana_low end,
  effects = { progressed = true },
  cost = function(f) return f.bags_full and 12 or 8 end,
  run = function()
    local best
    for _, u in ipairs(seen(100)) do
      if G.fits(u) and (not best or u.distance < best.distance) then best = u end
    end
    assert(best and Combat.engage(best.guid), "no mob in sight to fight")
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
      if G.fits(u) then
        if walking() then Move.free() end
        return true, true
      end
    end
    if walking() then return false end
    return true, false, "no mobs that give XP on the way"
  end,
  timeout = 120,
})

local travel = {}

G.action({
  name = "travel_to_zone",
  stop = function() Move.free() end,
  why = "Travel to a zone that suits our level (or the one a goal names).",
  pre = function()
    if pass.travel == nil then pass.travel = travel_target(top_zone_wish()) or false end
    return pass.travel ~= false
  end,
  effects = { zone_fit = "ok", mobs_for_xp = true },
  cost = function(f) return f.zone_fit == "ok" and 40 or 20 end,
  run = function(args)
    local z = travel_target(args.data and args.data.zone or top_zone_wish())
    assert(z, "no zone to travel to")
    args.subject = z.name
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
    return true, false, "could not reach " .. travel.zone.name .. " (" .. tostring(outcome) .. ")"
  end,
  timeout = 1800,
})

-- Everything defined so far is built in; an action the planner teaches (or redefines) is not.
for _, a in pairs(G.actions) do a.built_in = true end

G.push({ name = "progress", standing = true, priority = 0, want = { progressed = true },
  why = "Keep levelling: quests first, then training and upkeep, else grind, else find a better place." })

return G
