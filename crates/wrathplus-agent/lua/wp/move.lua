-- wp.move: getting to someone and staying with them, so the tiers above say who and where, not
-- how. Loaded for every agent as the global `Move`. A unit is a party token ("party1"), a name
-- ("Pallytest") or a guid from AgentNearby.
--
--   Move.to(unit [, within])    walk up to a unit, even a moving one (routing again as it moves),
--                               and stop within `within` yards (default: close enough to talk or
--                               trade). Ends as AGENT_MOVE_DONE (unit, "arrived" | "lost" | "failed").
--   Move.follow(unit)           follow mode: keep within `follow_distance` of them (moving only
--                               once they are `slack` yards further, as players do), go to where
--                               they were last seen if they leave sight, and fight beside them:
--                               their target when they are fighting (`assist`), and anything that
--                               attacks you. Following resumes after the fight.
--   Move.stay()                 hold this spot: fight what attacks you, then walk back.
--   Move.free()                 no mode; stop.
--   Move.mode, Move.state       "follow Pallytest" / "stay" / "to Marshal Dughan", and what it is
--                               doing right now; Move.reached() for an intent's `until`.
--
-- Grouped, the fast tier is offered two intents: follow_party_leader and stay_here.

local M = {
  mode = nil,
  state = nil,
  follow_distance = 3,
  slack = 3,
  within = 4,
  drift = 2.5,
  assist = true,
  -- Out of sight this long (seconds) while going to them: lost.
  lose_after = 30,
  last = nil,
}

local who, guid, anchor, aimed, seen, seen_at, started

local function far(a, b)
  return math.sqrt((a.x - b.x) ^ 2 + (a.y - b.y) ^ 2)
end

local function same(a, b)
  return a and b and string.lower(a) == string.lower(b)
end

-- The unit's guid now: a token or a guid answers directly, a name by what is in sight or in the party.
local function resolve(unit)
  if type(unit) ~= "string" then return nil end
  if unit:match("^0[xX]%x+$") then return unit end
  if UnitExists(unit) then return UnitGUID(unit) end
  for _, u in ipairs(AgentNearby(150)) do
    if same(u.name, unit) then return u.guid end
  end
  for i = 1, 4 do
    if same(UnitName("party" .. i), unit) then return UnitGUID("party" .. i) end
  end
end

local function find(g)
  if not g then return nil end
  for _, u in ipairs(AgentNearby(150)) do
    if same(u.guid, g) then return u end
  end
end

local function signal(...)
  if Agent and Agent.signal then Agent.signal(...) end
end

-- Walk toward a unit, aiming at the point `within` short of it on our side, and routing again
-- once it has moved on.
local function close_in(u, within)
  local me = AgentPosition()
  if not me then return end
  local d = math.max(far(me, u), 0.1)
  local stop = math.min(within * 0.6, d)
  local target = { x = u.x + (me.x - u.x) / d * stop, y = u.y + (me.y - u.y) / d * stop }
  if not aimed or far(aimed, target) > M.drift or AgentWalk().status ~= "walking" then
    AgentMoveTo(target.x, target.y, u.z)
    aimed = target
  end
end

local function halt()
  if AgentWalk().status == "walking" then AgentStop() end
  aimed = nil
end

local function finish(outcome)
  local name = who
  M.mode, M.state, who, guid, aimed = nil, nil, nil, nil, nil
  M.last = { unit = name, outcome = outcome }
  halt()
  signal("MOVE_DONE", name, outcome)
end

local function set(mode, unit)
  if Combat and Combat.active then Combat.stop() end
  who, guid, aimed, seen, seen_at, started = unit, resolve(unit), nil, nil, GetTime(), GetTime()
  M.mode, M.last = mode, nil
end

function M.to(unit, within)
  set("to", unit)
  M.to_within = within or M.within
  return guid ~= nil
end

function M.follow(unit)
  set("follow", unit)
  return guid ~= nil
end

function M.stay()
  set("stay", nil)
  anchor = AgentPosition()
  return true
end

function M.free()
  M.mode, M.state, who, guid, aimed = nil, nil, nil, nil, nil
  halt()
end

-- For an intent's `until`: whether the last Move.to is over, and whether it arrived.
function M.reached()
  if M.mode == "to" then return false end
  return true, M.last ~= nil and M.last.outcome == "arrived", M.last and M.last.outcome
end

-- Who to fight: something attacking us, else (assisting) the leader's target in their fight.
local function foe(leader)
  local me = UnitGUID("player")
  for _, u in ipairs(AgentNearby(40)) do
    if not u.dead and u.in_combat and u.kind == "unit" and same(u.target, me) then return u.guid end
  end
  if M.assist and leader and leader.in_combat and leader.target then
    local t = find(leader.target)
    if t and not t.dead and t.reaction ~= "friendly" and t.kind == "unit" then return t.guid end
  end
end

local function fight(leader)
  if not Combat then return false end
  if Combat.active then
    M.state = "fighting (" .. (Combat.state or "") .. ")"
    return true
  end
  local g = foe(leader)
  if g and Combat.engage(g) then
    M.state = "fighting"
    return true
  end
  return false
end

local function tick(now)
  if UnitIsDeadOrGhost("player") then
    M.state = "dead"
    return
  end
  if M.mode == "stay" then
    if fight(nil) then return end
    local me = AgentPosition()
    if anchor and me and far(me, anchor) > 4 then
      if AgentWalk().status ~= "walking" then AgentMoveTo(anchor.x, anchor.y, anchor.z) end
      M.state = "walking back to my spot"
    else
      M.state = "holding this spot"
    end
    return
  end
  guid = guid or resolve(who)
  local u = find(guid)
  if u then seen, seen_at = { x = u.x, y = u.y, z = u.z }, now end
  if M.mode == "to" then
    if u and u.distance <= M.to_within then return finish("arrived") end
    if u then
      close_in(u, M.to_within)
      M.state = string.format("going to %s (%d yd)", u.name or who, u.distance)
    elseif seen and now - seen_at < M.lose_after then
      if AgentWalk().status ~= "walking" then AgentMoveTo(seen.x, seen.y, seen.z) end
      M.state = "going to where " .. tostring(who) .. " was last seen"
    else
      return finish(seen and "lost" or "failed")
    end
    return
  end
  -- Follow.
  if fight(u) then return end
  if u then
    if u.distance > M.follow_distance + M.slack or (aimed and u.distance > M.follow_distance) then
      close_in(u, M.follow_distance)
      M.state = string.format("following %s (%d yd)", u.name or who, u.distance)
    else
      halt()
      M.state = "with " .. (u.name or who)
    end
  elseif seen then
    if AgentWalk().status ~= "walking" then AgentMoveTo(seen.x, seen.y, seen.z) end
    M.state = tostring(who) .. " is out of sight: going to where they were"
  else
    M.state = "looking for " .. tostring(who)
  end
end

local frame = CreateFrame("Frame")
local since = 0
frame:SetScript("OnUpdate", function(_, dt)
  since = since + dt
  if since < 0.25 then return end
  since = 0
  if M.mode then tick(GetTime()) end
end)

-- Grouped, the fast tier may put the agent in follow mode or have it hold a spot by itself.
if Agent and Agent.intent then
  local function leader()
    local i = GetPartyLeaderIndex and GetPartyLeaderIndex() or 0
    if (GetNumPartyMembers() or 0) > 0 and i > 0 then return "party" .. i end
  end
  Agent.intent("follow_party_leader", {
    when = function()
      return "In a party: follow the leader (" .. tostring(UnitName(leader() or "") or "?")
        .. "), staying close and fighting beside them."
    end,
    available = function() return leader() ~= nil end,
    run = function() M.follow(UnitName(leader())) end,
    done = function() return M.mode ~= "follow" end,
  })
  Agent.intent("stay_here", {
    when = "Wait at this spot: fight what attacks you, but don't wander.",
    available = function() return (GetNumPartyMembers() or 0) > 0 end,
    run = function() M.stay() end,
    done = function() return M.mode ~= "stay" end,
  })
end

return M
