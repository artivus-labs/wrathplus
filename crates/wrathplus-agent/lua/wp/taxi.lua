-- wp.taxi: flight paths, so a long trip flies where that beats the walk, and the paths on the way
-- are learnt. Loaded for every agent as the global `Taxi`, after `Goals`; `Taxi.on = false` walks
-- everywhere (and learns nothing).
--
--   Taxi.known()        the flight paths this character knows on this map, nearest first:
--                       { { id, name, x, y, z, distance }, ... }, and whether the client has read
--                       them this session (AgentTaxiNodes: a flight master's map lists them all;
--                       until one opens, only the paths learnt in sight count).
--   Taxi.plan(x, y[, z]) whether a trip there flies: { from, to, walk, fly, fare } (the nodes,
--                       the seconds each way is reckoned at, the fare last seen or nil), else nil,
--                       why. Move.travel(x, y[, z]) is Move.to that flies when this says so; the
--                       goal runner's long walks (hand_in_quests, visit_quest_hub, travel_to_zone)
--                       go by it.
--   Taxi.journey        the flight under way ({ stage, from, to, state }); Taxi.last how the last
--                       one ended ({ ok, why }); Taxi.log the last few lines.
--   Taxi.fares          [from][to] = copper, by node name, from the maps seen.
--
-- Flying: walk to the flight master at the first node, talk to it (a gossip greeting is answered
--   with its taxi option), take the node nearest the destination when the money over the reserve
--   (`reserve` copper, nil: Upkeep.reserve()) covers its fare, wait out the flight (UnitOnTaxi)
--   and walk the rest. Each flight ends as AGENT_FLIGHT_DONE(ok, why): "landed at <node>", or why
--   it walks instead ("no flight master at <node>", "the fare to <node> is 120 copper, 80 spare
--   over the reserve", ...), and the walk goes on from where it stands. A node whose flight failed
--   is left out of plans for `retry_after` seconds.
-- Deciding: a trip shorter than `min_trip` yards is walked. Otherwise the walk is reckoned at
--   `walk_speed` yd/s over the straight line times `detour`; the flight at the walk to the nearest
--   known node, `fly_speed` yd/s from node to node (straight, times `fly_detour`), the walk on from
--   the known node nearest the destination, and `overhead` seconds (talking, take-off, landing).
--   It flies when that saves `min_gain` seconds or more and the spare money covers the fare last
--   seen on that route (else `min_fare`).
-- Learning: the goal action learn_flight_path (fact flight_path_unknown_near) talks to a flight
--   master in sight within `learn_radius` yards whose node we do not know (the green "!" over it:
--   AgentNearby's taxi = "unknown"), or walks to a flight master the realm named (Goals.places
--   .flightmaster, wp.services) within `learn_detour` yards whose node is not known. Talking to it
--   learns the node, as a first visit does; AGENT_FLIGHT_PATH_LEARNED(name) says so. One talked
--   to is not walked to again for `learn_wait` seconds.

local M = {
  on = true,
  min_trip = 500, walk_speed = 7, detour = 1.3, fly_speed = 30, fly_detour = 1.3, overhead = 45,
  min_gain = 60, min_fare = 50, reserve = nil,
  -- A flight master serves the node within this many yards of it; one is looked for (and talked
  -- to) once we are this near its node.
  serves = 60, talk_reach = 40,
  -- Seconds: a map that has not opened is asked for again (`talk_tries` times in all), a flight
  -- that has not started is given up, a walk that gains no ground is given up.
  talk_wait = 10, talk_tries = 3, board_wait = 15, stall_after = 30,
  retry_after = 600,
  learn_radius = 100, learn_detour = 300, learn_wait = 1800,
  journey = nil, last = nil, fares = {}, log = {}, map = nil,
}

local failed, tried = {}, {}

local function now() return GetTime() end
local function far(a, b) return math.sqrt((a.x - b.x) ^ 2 + (a.y - b.y) ^ 2) end

local function note(text)
  M.log[#M.log + 1] = string.format("%.1f %s", now(), text)
  if #M.log > 20 then table.remove(M.log, 1) end
end

local function signal(...)
  if Agent and Agent.signal then Agent.signal(...) end
end

local function side_ok(n)
  local faction = UnitFactionGroup and UnitFactionGroup("player")
  if faction == "Alliance" then return n.alliance ~= false end
  if faction == "Horde" then return n.horde ~= false end
  return true
end

local function nodes()
  if not AgentTaxiNodes then return {}, false end
  local list, read = AgentTaxiNodes()
  return list or {}, read
end

function M.known()
  local list, read = nodes()
  local out = {}
  for _, n in ipairs(list) do
    if n.known and side_ok(n) then
      out[#out + 1] = { id = n.id, name = n.name, x = n.x, y = n.y, z = n.z, distance = n.distance }
    end
  end
  return out, read == true
end

local function spare()
  local reserve = M.reserve or (Upkeep and Upkeep.reserve and Upkeep.reserve()) or 0
  return (GetMoney and GetMoney() or 0) - reserve
end

local function nearest(list, p, skip)
  local best, best_d
  for _, n in ipairs(list) do
    if n ~= skip and not (failed[n.name] and now() < failed[n.name]) then
      local d = far(n, p)
      if not best_d or d < best_d then best, best_d = n, d end
    end
  end
  return best, best_d
end

function M.plan(x, y, z)
  local to = type(x) == "table" and x or { x = x, y = y, z = z }
  local pos = AgentPosition and AgentPosition()
  if not M.on then return nil, "Taxi.on is false" end
  if not pos then return nil, "not in the world" end
  if to.map and pos.map and to.map ~= pos.map then return nil, "the destination is on another map" end
  local d = far(pos, to)
  if d < M.min_trip then return nil, string.format("%d yards is a walk (min_trip %d)", d, M.min_trip) end
  local known = M.known()
  local from, d_from = nearest(known, pos)
  local dest, d_dest = nearest(known, to, from)
  if not from or not dest then return nil, "fewer than two known flight paths on this map to fly between" end
  local walk = d * M.detour / M.walk_speed
  local fly = (d_from + d_dest) * M.detour / M.walk_speed + far(from, dest) * M.fly_detour / M.fly_speed + M.overhead
  local plan = { from = from, to = dest, walk = math.floor(walk), fly = math.floor(fly),
    fare = M.fares[from.name] and M.fares[from.name][dest.name] }
  if walk - fly < M.min_gain then
    return nil, string.format("flying from %s to %s (%d s) saves under %d s of the %d s walk", from.name, dest.name, plan.fly, M.min_gain, plan.walk)
  end
  local money = spare()
  if money < (plan.fare or M.min_fare) then
    return nil, string.format("%d copper spare over the reserve, under the fare to %s (%d)", money, dest.name, plan.fare or M.min_fare)
  end
  return plan
end

---------------------------------------------------------------------------------------------------
-- The flight: Move.travel hands its journey to Taxi.step every tick, until it says to walk on.

local function close_windows()
  if NumTaxiNodes and NumTaxiNodes() > 0 and CloseTaxiMap then CloseTaxiMap() end
  if GetNumGossipOptions and GetNumGossipOptions() > 0 and CloseGossip then CloseGossip() end
end

local function over(ok, why)
  M.journey, M.last = nil, { ok = ok, why = why }
  note((ok and "flew: " or "walking instead: ") .. why)
  signal("FLIGHT_DONE", ok, why)
  return "walk"
end

-- Walk on instead: say why, and leave the node out of plans for a while.
local function fall_back(j, why, node)
  close_windows()
  failed[(node or j.from).name] = now() + M.retry_after
  return over(false, why)
end

local function master_at(node)
  for _, u in ipairs(AgentNearby and AgentNearby(100) or {}) do
    if not u.dead and u.npc and far(u, node) <= M.serves then
      for _, s in ipairs(u.npc) do
        if s == "flightmaster" then return u end
      end
    end
  end
end

function M.start(plan)
  local j = { stage = "walk", from = plan.from, to = plan.to, since = now(), tries = 0 }
  M.journey = j
  if UnitOnTaxi and UnitOnTaxi("player") then
    j.stage = "fly"
  else
    note(string.format("flying %s -> %s (%d s against %d s on foot)", plan.from.name, plan.to.name, plan.fly, plan.walk))
  end
  return j
end

-- Record the open map: the fares from its node, by name (TAXIMAP_OPENED).
function M.record()
  local n = NumTaxiNodes and NumTaxiNodes() or 0
  if n <= 0 then return end
  local here, fares = nil, {}
  for i = 1, n do
    local kind = TaxiNodeGetType(i)
    if kind == "CURRENT" then here = TaxiNodeName(i)
    elseif kind == "REACHABLE" then fares[TaxiNodeName(i)] = TaxiNodeCost(i) end
  end
  M.map = { at = now(), here = here, nodes = n }
  if here then M.fares[here] = fares end
end

local function talk(j, t)
  if NumTaxiNodes and NumTaxiNodes() > 0 then
    M.record()
    for i = 1, NumTaxiNodes() do
      if TaxiNodeName(i) == j.to.name then
        if TaxiNodeGetType(i) ~= "REACHABLE" then break end
        local fare, money = TaxiNodeCost(i) or 0, spare()
        if fare > money then
          return fall_back(j, string.format("the fare to %s is %d copper, %d spare over the reserve", j.to.name, fare, money), j.to)
        end
        TakeTaxiNode(i)
        j.stage, j.since = "board", t
        return nil
      end
    end
    return fall_back(j, j.to.name .. " is not on " .. j.from.name .. "'s map: no route there", j.to)
  end
  if not j.gossiped and GetNumGossipOptions and GetNumGossipOptions() > 0 then
    j.gossiped = true
    local options = { GetGossipOptions() }
    for i = 1, #options / 2 do
      if options[i * 2] == "taxi" then
        SelectGossipOption(i)
        return nil
      end
    end
    return fall_back(j, "the flight master at " .. j.from.name .. " offers no flight")
  end
  if t - j.since < M.talk_wait then return nil end
  j.tries = j.tries + 1
  local u = master_at(j.from)
  if j.tries >= M.talk_tries or not u then return fall_back(j, "the flight master's map at " .. j.from.name .. " never opened") end
  AgentInteract(u.guid)
  j.since, j.gossiped = t, false
  return nil
end

-- The walk to the flight master: up to it once it is in sight near the node.
local function walk(j, t)
  local u = master_at(j.from)
  if u and u.distance <= M.talk_reach then
    AgentInteract(u.guid)
    j.stage, j.since, j.tries = "talk", t, 0
    return nil
  end
  local pos = AgentPosition()
  if not pos then return nil end
  if not j.sent then
    j.sent = true
    if far(pos, j.from) <= M.talk_reach and not u then return fall_back(j, "no flight master at " .. j.from.name) end
    local route, why = AgentMoveTo(j.from.x, j.from.y, j.from.z)
    if not route then return fall_back(j, "no way to the flight master at " .. j.from.name .. ": " .. tostring(why)) end
    j.progress = { x = pos.x, y = pos.y, at = t }
    return nil
  end
  local status = AgentWalk().status
  if status == "stopped" or status == "idle" then return "interrupted" end
  if status == "stuck" or status == "failed" then
    return fall_back(j, "could not reach the flight master at " .. j.from.name .. " (" .. status .. ")")
  end
  if status == "arrived" then return fall_back(j, "no flight master at " .. j.from.name) end
  local fighting = UnitAffectingCombat("player") or (Combat and Combat.active)
  if fighting or far(pos, j.progress) > 2 then
    j.progress = { x = pos.x, y = pos.y, at = t }
  elseif t - j.progress.at > M.stall_after then
    return fall_back(j, "the walk to the flight master at " .. j.from.name .. " stalled")
  end
  return nil
end

-- One tick of the journey: nil while under way, "walk" when the walk to the destination goes on
-- (landed, or flying fell through), "interrupted" when someone else stopped the walk.
function M.step(j, t)
  if j ~= M.journey then return "interrupted" end
  if j.stage == "walk" then
    j.state = "going to the flight master at " .. j.from.name
    return walk(j, t)
  elseif j.stage == "talk" then
    j.state = "talking to the flight master at " .. j.from.name
    return talk(j, t)
  elseif j.stage == "board" then
    j.state = "taking the flight to " .. j.to.name
    if UnitOnTaxi("player") then
      j.stage = "fly"
    elseif t - j.since > M.board_wait then
      return fall_back(j, "the flight to " .. j.to.name .. " did not start", j.to)
    end
    return nil
  end
  j.state = "flying to " .. j.to.name
  if UnitOnTaxi("player") then return nil end
  return over(true, "landed at " .. j.to.name)
end

-- The journey's walk was dropped (Move.free, another Move order): close what it left open.
function M.cancel(j)
  if j ~= M.journey then return end
  if j.stage == "talk" or j.stage == "board" then close_windows() end
  M.journey = nil
end

local frame = CreateFrame("Frame")
frame:RegisterEvent("TAXIMAP_OPENED")
frame:SetScript("OnEvent", function() M.record() end)

---------------------------------------------------------------------------------------------------
-- Learning: a goal action

-- The flight master near us whose node we do not know: in sight, else a place the realm named.
function M.unknown_near()
  if not M.on then return nil end
  local t = now()
  local function fresh(name) return not (tried[name] and t < tried[name]) end
  for _, u in ipairs(AgentNearby and AgentNearby(M.learn_radius) or {}) do
    if u.taxi == "unknown" and not u.dead and u.reaction ~= "hostile" and fresh(u.name) then
      return { name = u.name, guid = u.guid, x = u.x, y = u.y, z = u.z, distance = u.distance }
    end
  end
  local pos = AgentPosition and AgentPosition()
  local places = Goals and Goals.places and Goals.places.flightmaster
  if not pos or not places or #places == 0 then return nil end
  local list
  for _, p in ipairs(places) do
    local d = far(pos, p)
    if (not p.map or p.map == pos.map) and d <= M.learn_detour and fresh(p.name) then
      list = list or nodes()
      local n = nearest(list, p)
      if n and far(n, p) <= M.serves and not n.known and side_ok(n) then
        return { name = p.name, guid = p.guid, x = p.x, y = p.y, z = p.z, distance = d, map = p.map }
      end
    end
  end
end

if Goals then
  local G = Goals
  G.fact("flight_path_unknown_near", function() return M.unknown_near() ~= nil end)

  local learning = {}
  local function learned()
    if M.map and M.map.at >= learning.since then return true end
    for _, u in ipairs(AgentNearby and AgentNearby(M.learn_radius) or {}) do
      if u.name == learning.place.name and u.taxi == "known" then return true end
    end
    return false
  end

  -- Built in like Goals' own: the zone guard is for actions the planner teaches.
  G.action({
    name = "learn_flight_path", built_in = true,
    why = "Talk to a flight master near us whose flight path we do not know yet: it learns the path, to fly later.",
    pre = function(f) return f.flight_path_unknown_near and not f.in_combat end,
    effects = { flight_path_unknown_near = false, progressed = true },
    cost = function()
      local u = M.unknown_near()
      return 1.5 + (u and math.min((u.distance or 0) / 200, 2) or 0)
    end,
    stop = function()
      G.ask_gossip(nil)
      if Move then Move.free() end
    end,
    run = function(args)
      local u = M.unknown_near()
      assert(u, "no flight master with an unknown path near")
      args.subject, args.going = u.name, "the flight master " .. u.name
      learning = { place = u, since = now() }
      tried[u.name] = now() + M.learn_wait
      note("learning the flight path at " .. u.name)
      -- A master whose path we know after all greets with gossip: its map then lists them all.
      G.ask_gossip("taxi")
    end,
    done = function()
      if learned() then
        G.ask_gossip(nil)
        close_windows()
        signal("FLIGHT_PATH_LEARNED", learning.place.name)
        return true, true
      end
      local finished, ok, why = G.visit(learning)
      if finished then G.ask_gossip(nil) end
      return finished, ok, why
    end,
    timeout = 120,
  })
end

return M
