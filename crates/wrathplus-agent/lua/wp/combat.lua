-- wp.combat: fighting a target, kept in range. Loaded for every agent as the global `Combat`.
--
--   Combat.engage(guid)  target the unit, chase it into range through the navmesh (routing again
--                        as it moves), face it, auto-attack and cast the Rotation (wp.rotation),
--                        until it dies, is lost, or Combat.stop() is called. Returns true if the
--                        unit is in sight. Casters and hunters fight from `reach` yards (their
--                        class's range), facing the target without walking in.
--   Combat.active        the guid being fought, or nil. Combat.state says what it is doing.
--   Combat.engaged(guid) for an intent's done: true once that fight is over (and the outcome).
--
-- The end of a fight is signalled as AGENT_COMBAT_DONE (guid, "killed" | "lost" | "stopped" |
-- "died"). Combat.history keeps the last fights ({ name, outcome, seconds, hp_lost }); every
-- `review_every` fights, and after a death, AGENT_COMBAT_REVIEW carries a summary with the
-- rotation's casts and failures, for the planner to rework Rotation.list.

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
  history = {},
  fights = 0,
  review_every = 10,
  -- Set reach from the class on each fight (casters and hunters keep their distance); false keeps
  -- a reach of your own.
  auto_reach = true,
}

local name_of, started, hp_at_start, faced

local aimed, swung, seen_at

local function find(guid)
  for _, u in ipairs(AgentNearby(80)) do
    if u.guid == guid then return u end
  end
end

local function far(a, b)
  return math.sqrt((a.x - b.x) ^ 2 + (a.y - b.y) ^ 2)
end

-- A summary of the last fights and the rotation's record over them, for the planner.
local function review(reason)
  local n, won, died, seconds = 0, 0, 0, 0
  local from = math.max(1, #C.history - C.review_every + 1)
  for i = from, #C.history do
    local f = C.history[i]
    n, seconds = n + 1, seconds + f.seconds
    if f.outcome == "killed" then won = won + 1 elseif f.outcome == "died" then died = died + 1 end
  end
  local spells = {}
  for spell, st in pairs(Rotation and Rotation.stats or {}) do
    spells[#spells + 1] = string.format("%s %d cast%s", spell, st.cast, st.failed > 0 and (", " .. st.failed .. " failed") or "")
  end
  table.sort(spells)
  local text = string.format(
    "%s. Last %d fights: %d won, %d died, %ds on average. Rotation (%s): %s.",
    reason, n, won, died, n > 0 and math.floor(seconds / n) or 0,
    Rotation and Rotation.describe() or "none", #spells > 0 and table.concat(spells, "; ") or "no spells cast")
  if Rotation then Rotation.reset_stats() end
  return text
end

local function done(outcome)
  local guid = C.active
  local seconds = started and math.floor(GetTime() - started) or 0
  local hp_lost = (hp_at_start or 0) - (UnitHealth("player") or 0)
  C.active, C.state, aimed, swung, seen_at, faced = nil, nil, nil, nil, nil, nil
  C.last = { guid = guid, outcome = outcome }
  if outcome ~= "stopped" then
    C.history[#C.history + 1] = { name = name_of, outcome = outcome, seconds = seconds, hp_lost = hp_lost }
    if #C.history > 20 then table.remove(C.history, 1) end
    C.fights = C.fights + 1
  end
  if AgentStop then AgentStop() end
  if Agent and Agent.signal then
    Agent.signal("COMBAT_DONE", guid, outcome)
    if outcome == "died" then
      Agent.signal("COMBAT_REVIEW", review("You died fighting " .. tostring(name_of)))
    elseif outcome ~= "stopped" and C.fights % C.review_every == 0 then
      Agent.signal("COMBAT_REVIEW", review("Fight review"))
    end
  end
end

function C.engage(guid)
  local u = find(guid)
  if not u then return false end
  C.active, aimed, swung, seen_at, faced = guid, nil, nil, GetTime(), nil
  C.last = nil
  name_of, started, hp_at_start = u.name, GetTime(), UnitHealth("player")
  if C.auto_reach then
    local _, class = UnitClass("player")
    local range = Rotation and Rotation.RANGE and Rotation.RANGE[class or ""]
    C.reach, C.ranged = range or 3, range ~= nil
  end
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
  if AgentWalk().status == "walking" then AgentStop() end
  aimed = nil
  if C.ranged then
    -- From range: turn to it (a cast needs facing), without the attack's walk into melee.
    if not faced or now - faced >= 1 then
      faced = now
      if AgentFace then AgentFace(C.active) end
    end
  elseif not swung or now - swung >= 1.5 then
    -- In melee: the right-click attack faces the unit and starts auto-attack.
    swung = now
    AgentInteract(C.active)
  end
  -- A rotation's bug must not stop the fight: keep swinging, and say what broke.
  local ok, cast = true, nil
  if Rotation then ok, cast = pcall(Rotation.tick, u, started) end
  if not ok then
    C.rotation_error = tostring(cast)
    cast = nil
  end
  C.state = "fighting " .. (u.name or "the target") .. (cast and (": " .. cast) or "")
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
