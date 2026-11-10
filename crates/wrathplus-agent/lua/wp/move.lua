-- wp.move: getting to someone and staying with them, so the tiers above say who and where, not
-- how. Loaded for every agent as the global `Move`. A unit is a party token ("party1"), a name
-- ("Pallytest", "Marshal Dughan") or a guid from AgentNearby.
--
--   Move.to(unit [, within])    walk up to a unit, even a moving one (routing again as it moves),
--                               and stop within `within` yards (default: close enough to talk or
--                               trade). Ends as AGENT_MOVE_DONE (target, "arrived" | "lost" | "failed");
--                               following ends that way too, "offline", when they log off.
--   Move.to(x, y [, z])         the same for a place (or a table { x =, y = [, z =] }): the navmesh
--                               route, ending as AGENT_MOVE_DONE too. A name not in sight fails at
--                               once (false, why): the client only knows what it can see. A walk
--                               someone stops short (AgentStop) ends "interrupted"; one that gains
--                               no ground (`stall_yards` in `stall_after` seconds, out of a fight and
--                               not eating) is sent again once, then ends "stalled", which is also
--                               put out as AGENT_WALK_DONE("stalled", target).
--   Move.follow(unit)           follow mode: keep within `follow_distance` of them (moving only
--                               once they are `slack` yards further, as players do), go to where
--                               they were last seen if they leave sight, and fight beside them:
--                               their target when they are fighting (`assist`), and anything that
--                               attacks you. Following resumes after the fight.
--   Move.stay()                 hold this spot: fight what attacks you, then walk back.
--   Move.free()                 no mode; stop.
--   Move.face_speakers          (default true) turn to whoever talks to you from close by (a party
--                               line, a whisper, an emote at you, a line naming you) while you are
--                               not fighting or walking, as a player looks at them.
--   Move.mode, Move.state       "follow Pallytest" / "stay" / "to Marshal Dughan", and what it is
--                               doing right now; Move.reached() for an intent's `until`.
--
-- Under a cinematic (AgentCinematic) nothing is in sight and no walk starts: a mode waits for it
-- to end, and a Move.to(x, y) asked for meanwhile fails at once with the reason.
--
-- Grouped, the fast tier is offered two intents: follow_companion (Move.companion if set, else the
-- party leader, else, when this agent leads, the first other member) and stay_here.

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
  -- Whom the group's follow intent follows (a name); nil: the leader, or the first other member.
  companion = nil,
  face_speakers = true,
  -- A walk that has not gained `stall_yards` in `stall_after` seconds has stalled.
  stall_yards = 2, stall_after = 20,
}

local who, guid, anchor, aimed, seen, seen_at, started, point
-- Where the walk last gained ground and when, and whether it was sent again after stalling.
local progress, resent

local function far(a, b)
  return math.sqrt((a.x - b.x) ^ 2 + (a.y - b.y) ^ 2)
end

-- A walk under way, or one whose route is still coming from the realm.
local function going()
  local status = AgentWalk().status
  return status == "walking" or status == "routing"
end

local function same(a, b)
  return a and b and string.lower(a) == string.lower(b)
end

local function online(unit)
  return UnitExists(unit) and (not UnitIsConnected or UnitIsConnected(unit))
end

-- A party member by name who has logged off (not one merely out of sight).
local function logged_off(name)
  for i = 1, 4 do
    local unit = "party" .. i
    if same(UnitName(unit), name) then return not online(unit) end
  end
  return false
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

-- Stop our own walk only: a walk someone else started (AgentInteract to an innkeeper while
-- following) is theirs to finish.
local function halt()
  if aimed and going() then AgentStop() end
  aimed = nil
end

local function finish(outcome)
  local name = who
  M.mode, M.state, who, guid, aimed, point = nil, nil, nil, nil, nil, nil
  M.last = { unit = name, outcome = outcome }
  halt()
  signal("MOVE_DONE", name, outcome)
end

local function set(mode, unit)
  if Combat and Combat.active then Combat.stop() end
  who, guid, aimed, seen, seen_at, started = unit, resolve(unit), nil, nil, GetTime(), GetTime()
  progress, resent = nil, false
  M.mode, M.last = mode, nil
end

function M.to(unit, within, z)
  -- A place: Move.to(x, y [, z]) or Move.to({ x =, y = [, z =] }).
  if type(unit) == "number" or (type(unit) == "table" and unit.x) then
    local p = type(unit) == "table" and unit or { x = unit, y = within, z = z }
    set("to", string.format("(%d, %d)", p.x, p.y))
    point, M.to_within = p, 3
    local route, why = AgentMoveTo(p.x, p.y, p.z)
    if not route then
      finish("failed")
      return false, why
    end
    return true
  end
  set("to", unit)
  point, M.to_within = nil, within or M.within
  if not guid then
    finish("failed")
    return false, tostring(unit) .. " is not in sight: walk closer, or give coordinates"
  end
  return true
end

-- The NPC windows following waits on, and how to close each.
local WINDOWS = {
  { "GossipFrame", "CloseGossip" }, { "MerchantFrame", "CloseMerchant" }, { "QuestFrame", "CloseQuest" },
  { "ClassTrainerFrame", "CloseTrainer" }, { "TaxiFrame", "CloseTaxiMap" },
}

local function window_open()
  for _, w in ipairs(WINDOWS) do
    local f = _G[w[1]]
    if f and f.IsShown and f:IsShown() then return true end
  end
  return false
end

local function close_windows()
  for _, w in ipairs(WINDOWS) do
    local f, close = _G[w[1]], _G[w[2]]
    if f and f.IsShown and f:IsShown() and close then close() end
  end
end

local window_since

-- Asked to follow: leave whatever window is open and go.
function M.follow(unit)
  set("follow", unit)
  close_windows()
  window_since = nil
  return guid ~= nil
end

function M.stay()
  set("stay", nil)
  anchor = AgentPosition()
  return true
end

function M.free()
  halt()
  M.mode, M.state, who, guid, aimed = nil, nil, nil, nil, nil
end

-- For an intent's `until`: whether the last Move.to is over, and whether it arrived.
function M.reached()
  if M.mode == "to" then return false end
  return true, M.last ~= nil and M.last.outcome == "arrived", M.last and M.last.outcome
end

-- Eating or drinking: a walk that stands still meanwhile has not stalled.
function M.resting()
  return UnitBuff ~= nil and (UnitBuff("player", "Food") or UnitBuff("player", "Drink")) ~= nil
end

-- Whether the walk has stalled for good: no ground gained for `stall_after` seconds, out of a fight
-- and not resting. The first stall sends the walk again; the second is the end.
local function stalled(now)
  local me = AgentPosition()
  if not me then return false end
  local paused = UnitAffectingCombat("player") or (Combat and Combat.active) or M.resting()
  if paused or not progress or far(me, progress) > M.stall_yards then
    progress = { x = me.x, y = me.y, at = now }
    return false
  end
  if now - progress.at < M.stall_after then return false end
  progress.at = now
  if resent then return true end
  resent = true
  if point then
    AgentMoveTo(point.x, point.y, point.z)
  else
    aimed = nil
  end
  return false
end

local function stall_end()
  local name = who
  finish("stalled")
  signal("WALK_DONE", "stalled", name)
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
  -- Their fight is ours: no pull check on a mob the companion is already on.
  if g and Combat.engage(g, { force = true }) then
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
  -- A cinematic hides everyone and stops all walking: wait, with nobody counted as lost.
  local held = AgentCinematic and AgentCinematic()
  if held then
    seen_at = now
    M.state = "waiting: " .. held
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
  if M.mode == "to" and point then
    local walk = AgentWalk()
    local me = AgentPosition()
    if me and far(me, point) <= M.to_within then return finish("arrived") end
    if walk.status == "arrived" then return finish("arrived") end
    if walk.status == "stuck" or walk.status == "failed" then return finish("failed") end
    -- Stopped short by someone else (AgentStop): over, but no failure of the walk's.
    if walk.status == "stopped" or walk.status == "idle" then return finish("interrupted") end
    if stalled(now) then return stall_end() end
    M.state = string.format("going to %s (%d yd)", who, me and far(me, point) or 0)
    return
  end
  guid = guid or resolve(who)
  local u = find(guid)
  if u then seen, seen_at = { x = u.x, y = u.y, z = u.z }, now end
  if M.mode == "to" then
    if u and u.distance <= M.to_within then return finish("arrived") end
    if stalled(now) then return stall_end() end
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
  -- They logged off: following is over (the fast tier chooses again).
  if not u and logged_off(who) then return finish("offline") end
  -- Something else is walking us (AgentInteract to an innkeeper) or we are talking to someone:
  -- let it finish, as a player stops following to deal with an NPC.
  if not aimed and going() then
    M.state = "letting another walk finish"
    return
  end
  -- A window someone is using (quest work) holds following; one left open is closed after a
  -- few seconds, or following would wait on it for ever (stuck at a trainer's window).
  if window_open() then
    window_since = window_since or now
    if (Quests and Quests.busy and Quests.busy()) or now - window_since < 5 then
      M.state = "talking to someone; following again after"
      return
    end
    close_windows()
  end
  window_since = nil
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

-- Someone close talks to us: turn to them, unless something else has us.
local SPEECH = { "CHAT_MSG_SAY", "CHAT_MSG_YELL", "CHAT_MSG_PARTY", "CHAT_MSG_PARTY_LEADER",
  "CHAT_MSG_WHISPER", "CHAT_MSG_TEXT_EMOTE", "CHAT_MSG_EMOTE", "CHAT_MSG_MONSTER_SAY" }
local function for_us(event, text, sender)
  local lower, mine = string.lower(text or ""), string.lower(UnitName("player") or "")
  if event == "CHAT_MSG_PARTY" or event == "CHAT_MSG_PARTY_LEADER" or event == "CHAT_MSG_WHISPER" then return true end
  if (event == "CHAT_MSG_TEXT_EMOTE" or event == "CHAT_MSG_EMOTE") and lower:find(" you", 1, true) then return true end
  if mine ~= "" and lower:find(mine, 1, true) then return true end
  return event == "CHAT_MSG_SAY" and UnitInParty and UnitInParty(sender) and true or false
end

local listener = CreateFrame("Frame")
for _, event in ipairs(SPEECH) do listener:RegisterEvent(event) end
listener:SetScript("OnEvent", function(_, event, text, sender)
  if not M.face_speakers or type(sender) ~= "string" or same(sender, UnitName("player")) then return end
  if UnitAffectingCombat("player") or (Combat and Combat.active) or going() then return end
  if not for_us(event, text, sender) then return end
  for _, u in ipairs(AgentNearby(20)) do
    if same(u.name, sender) then return AgentFace(u.guid) end
  end
end)

-- Whom the group follows: the companion named, else the leader, else (leading ourselves, or the
-- leader logged off) the first other member who is online; nil when nobody is.
function M.companion_now()
  local n = GetNumPartyMembers() or 0
  if n == 0 then return nil end
  if M.companion then
    for i = 1, 4 do
      local unit = "party" .. i
      if same(UnitName(unit), M.companion) and online(unit) then return UnitName(unit) end
    end
  end
  local i = GetPartyLeaderIndex and GetPartyLeaderIndex() or 0
  if i > 0 and online("party" .. i) then return UnitName("party" .. i) end
  for j = 1, n do
    if online("party" .. j) then return UnitName("party" .. j) end
  end
end

-- Grouped, the fast tier may put the agent in follow mode or have it hold a spot by itself.
if Agent and Agent.intent then
  Agent.intent("follow_companion", {
    when = function()
      return "In a party: follow " .. tostring(M.companion_now() or "?")
        .. ", staying close and fighting beside them."
    end,
    available = function() return M.companion_now() ~= nil end,
    run = function() M.follow(M.companion_now()) end,
    done = function() return M.mode ~= "follow" end,
  })
  -- Run when someone asks (the planner, or the party through triage), never offered to the fast
  -- tier: at a fight's start "fight what attacks you" read like a combat choice and stopped it
  -- following.
  Agent.intent("stay_here", {
    when = "Wait at this spot because you were asked to: fight what attacks you, but don't wander.",
    available = function() return false end,
    run = function() M.stay() end,
    done = function() return M.mode ~= "stay" end,
  })
end

return M
