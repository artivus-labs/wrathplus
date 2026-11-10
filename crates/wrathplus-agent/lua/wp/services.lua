-- wp.services: where services are, from the realm (AgentServices, mod-agent-nav), handed to the
-- goal runner as places it knows, the way a player knows a town. Loaded for every agent as the
-- global `Services`.
--
-- On entering the world, on reaching a new zone, and every `every` seconds, the realm is asked for
-- the `kinds` within `radius`; the answer is kept in Services.places and handed to
-- Goals.learn_places(places) (one table per place: kind, name, x, y, z, map, zone; plus entry,
-- members and profession). A realm without the module says so once (Services.why), and is asked
-- again only in the next zone.
--
--   Services.on      false: never ask.
--   Services.places  the last answer, nearest first.
--   Services.ask()   ask now (after a long walk, say).

local M = {
  on = true,
  every = 300,
  radius = nil,
  kinds = {
    "vendor", "repair", "trainer", "flightmaster", "innkeeper", "banker", "questgiver-hub",
    "trainer_cooking", "trainer_first_aid", "trainer_profession",
  },
  places = {},
  why = nil,
  learned = 0,
}

local waiting, asked_at, asked_zone = false, nil, nil

-- The goal runner's table of known places (Goals.learn_places, agents/planner-guardrails #152);
-- without it the places are only kept here.
local function learn(places)
  M.places = places
  if Goals and Goals.learn_places then
    Goals.learn_places(places)
    M.learned = M.learned + 1
  end
  if Agent and Agent.signal then Agent.signal("SERVICES_LEARNED", #places) end
end

-- One call: the answer once it is in, else keep waiting.
local function poll(now, zone)
  local answer, why = AgentServices(M.kinds, M.radius)
  if not answer then
    waiting, asked_at, asked_zone, M.why = false, now, zone, why
    return
  end
  if answer.pending then
    waiting = true
    return
  end
  waiting, asked_at, asked_zone, M.why = false, now, zone, nil
  learn(answer)
end

function M.ask()
  asked_at = nil
end

local function tick(now)
  if not M.on or not AgentServices or not AgentPosition() then return end
  local zone = GetRealZoneText and GetRealZoneText() or ""
  local due = waiting or asked_at == nil or zone ~= asked_zone
    or (M.why == nil and now - asked_at >= M.every)
  if due then poll(now, zone) end
end

local frame = CreateFrame("Frame")
local since = 0
frame:SetScript("OnUpdate", function(_, dt)
  since = since + dt
  if since < 1 then return end
  since = 0
  tick(GetTime())
end)
M._tick = tick

return M
