-- wp.combat: melee that keeps up with its target. Loaded for every agent as the global `Combat`.
--
--   Combat.engage(guid)  target the unit, chase it into melee range through the navmesh (routing
--                        again as it moves), face it and auto-attack, until it dies, is lost, or
--                        Combat.stop() is called. Returns true if the unit is in sight.
--   Combat.active        the guid being fought, or nil. Combat.state says what it is doing.
--   Combat.engaged(guid) for an intent's done: true once that fight is over (and the outcome).
--
-- A spell rotation is the agent's: cast from its own code while Combat keeps it in range. The
-- end of a fight is signalled as AGENT_COMBAT_DONE (guid, "killed" | "lost" | "stopped" | "died").

local C = {
  active = nil,
  state = nil,
  -- Melee range in yards, below the server's (5 plus both units' combat reach).
  reach = 3,
  -- Route again once the target has moved this far from where the walk was aimed.
  drift = 2.5,
  -- Give up on a unit out of sight this long.
  lose_after = 5,
  last = nil,
}

local aimed, swung, seen_at

local function find(guid)
  for _, u in ipairs(AgentNearby(80)) do
    if u.guid == guid then return u end
  end
end

local function far(a, b)
  return math.sqrt((a.x - b.x) ^ 2 + (a.y - b.y) ^ 2)
end

local function done(outcome)
  local guid = C.active
  C.active, C.state, aimed, swung, seen_at = nil, nil, nil, nil, nil
  C.last = { guid = guid, outcome = outcome }
  if AgentStop then AgentStop() end
  if Agent and Agent.signal then Agent.signal("COMBAT_DONE", guid, outcome) end
end

function C.engage(guid)
  local u = find(guid)
  if not u then return false end
  C.active, aimed, swung, seen_at = guid, nil, nil, GetTime()
  C.last = nil
  AgentTarget(guid)
  return true
end

function C.stop()
  if C.active then done("stopped") end
end

-- For an intent's `done`: whether the fight with `guid` (the active one if nil) is over.
function C.engaged(guid)
  if C.active and (guid == nil or C.active == guid) then return false end
  local last = C.last
  return true, last and last.outcome == "killed", last and last.outcome
end

local function tick(now)
  if UnitIsDeadOrGhost("player") then return done("died") end
  local u = find(C.active)
  if not u then
    if now - (seen_at or now) >= C.lose_after then return done("lost") end
    C.state = "looking for the target"
    return
  end
  seen_at = now
  if u.dead then return done("killed") end
  if u.distance > C.reach then
    -- Chase: aim at where it stands, and again whenever it has moved on.
    if not aimed or far(aimed, u) > C.drift or AgentWalk().status ~= "walking" then
      AgentMoveTo(u.x, u.y, u.z)
      aimed = { x = u.x, y = u.y }
    end
    swung = nil
    C.state = string.format("closing on %s (%d yd)", u.name or "the target", u.distance)
    return
  end
  -- In reach: the right-click attack faces the unit and starts auto-attack.
  if AgentWalk().status == "walking" then AgentStop() end
  aimed = nil
  if not swung or now - swung >= 1.5 then
    swung = now
    AgentInteract(C.active)
  end
  C.state = "fighting " .. (u.name or "the target")
end

local frame = CreateFrame("Frame")
local since = 0
frame:RegisterEvent("UI_ERROR_MESSAGE")
frame:SetScript("OnEvent", function(_, _, message)
  -- The server says too far or facing away: take it as a cue to close in and face again.
  if C.active and type(message) == "string" and (message:find("too far") or message:find("in front")) then
    aimed, swung = nil, nil
  end
end)
frame:SetScript("OnUpdate", function(_, dt)
  since = since + dt
  if since < 0.2 then return end
  since = 0
  if C.active then tick(GetTime()) end
end)

return C
