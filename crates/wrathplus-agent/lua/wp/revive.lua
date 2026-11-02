-- wp.revive: getting back up after a death, as a player does by reflex, without waiting on the
-- tiers above. Loaded for every agent as the global `Revive`; `Revive.on = false` hands deaths to
-- the agent's own code.
--
--   Dead:  accept a resurrection that is offered (a party healer, a soulstone is the agent's call);
--          otherwise release (RepopMe) after `release_after` seconds, longer in a party so a
--          healer has the chance to resurrect first.
-- `clear` and `safe` (yards) are fields, so code can tune them live.
--   Ghost: run back toward the corpse (AgentCorpse) through the navmesh, to a spot clear of the
--          hostiles standing over it; once the server's reclaim delay is over and nothing hostile
--          is close, RetrieveCorpse.
--
-- `Revive.state` says what it is doing ("waiting 21s for the reclaim delay", ...), nil when alive.
-- A death also ends the running activity (Agent.stop), so the fast tier chooses afresh after.

local R = {
  on = true,
  release_after = 3,
  party_release_after = 20,
  -- The server reclaims within 39 yards of the corpse; stay inside it.
  reach = 32,
  -- How far from the corpse's hostiles to stand up, and how close one may be: a monster a few
  -- levels up aggroes from about 20 yards, and they wander.
  clear = 30,
  safe = 25,
  state = nil,
}

local died_at, spot, tried_at

local function say(state)
  R.state = state
end

local function far(a, b)
  return math.sqrt((a.x - b.x) ^ 2 + (a.y - b.y) ^ 2)
end

-- Hostile, living units within `yards` of point `p`.
local function hostiles_near(p, yards)
  local out = {}
  for _, u in ipairs(AgentNearby(80)) do
    if u.reaction == "hostile" and not u.dead and u.kind ~= "corpse" and far(u, p) <= yards then
      out[#out + 1] = u
    end
  end
  return out
end

-- Where to stand up: the corpse, or a point `clear` yards from it, away from what stands over it.
local function choose_spot(corpse)
  local over = hostiles_near(corpse, R.clear)
  if #over == 0 then return { x = corpse.x, y = corpse.y, z = corpse.z } end
  local cx, cy = 0, 0
  for _, u in ipairs(over) do cx, cy = cx + u.x, cy + u.y end
  cx, cy = cx / #over, cy / #over
  local dx, dy = corpse.x - cx, corpse.y - cy
  local len = math.sqrt(dx * dx + dy * dy)
  if len < 0.1 then dx, dy, len = 1, 0, 1 end
  return { x = corpse.x + dx / len * R.clear, y = corpse.y + dy / len * R.clear, z = corpse.z }
end

local function ghost(now)
  local corpse = AgentCorpse and AgentCorpse()
  if not corpse then return say("ghost, waiting for the corpse's location") end
  local me = AgentPosition()
  if not me then return end
  if not spot or far(spot, corpse) > R.reach + 1 then spot = choose_spot(corpse) end
  local to_corpse = far(me, corpse)
  if to_corpse > R.reach or far(me, spot) > 6 then
    local walk = AgentWalk()
    if walk.status ~= "walking" then
      AgentMoveTo(spot.x, spot.y, spot.z)
      spot.started = now
    elseif spot.started and now - spot.started > 60 then
      spot = nil -- a minute without arriving: pick again
    end
    if to_corpse > R.reach then return say(string.format("ghost, running to the corpse (%d yd)", to_corpse)) end
  end
  local delay = GetCorpseRecoveryDelay() or 0
  if delay > 0 then return say(string.format("ghost at the corpse, waiting %ds for the reclaim delay", delay)) end
  local close = hostiles_near(me, R.safe)
  if #close > 0 then
    spot = nil
    return say("ghost at the corpse, waiting for " .. (close[1].name or "a hostile") .. " to move off")
  end
  if not tried_at or now - tried_at >= 3 then
    tried_at = now
    say("reclaiming the corpse")
    RetrieveCorpse()
  end
end

local frame = CreateFrame("Frame")
local since = 0
frame:RegisterEvent("PLAYER_DEAD")
frame:RegisterEvent("RESURRECT_REQUEST")
frame:SetScript("OnEvent", function(_, event)
  if not R.on then return end
  if event == "PLAYER_DEAD" then
    died_at, spot, tried_at = GetTime(), nil, nil
    if Agent and Agent.stop then Agent.stop("died") end
  elseif event == "RESURRECT_REQUEST" then
    AcceptResurrect()
    say("accepted a resurrection")
  end
end)
frame:SetScript("OnUpdate", function(_, dt)
  since = since + dt
  if since < 0.5 then return end
  since = 0
  if not R.on then
    R.state = nil
    return
  end
  local now = GetTime()
  if UnitIsGhost("player") then return ghost(now) end
  if UnitIsDead("player") then
    died_at = died_at or now
    local wait = (GetNumPartyMembers() or 0) > 0 and R.party_release_after or R.release_after
    if now - died_at >= wait then
      say("released")
      RepopMe()
      died_at = now -- once per wait, should the release not take
    else
      say(string.format("dead, releasing in %ds", math.ceil(wait - (now - died_at))))
    end
    return
  end
  died_at, spot, tried_at, R.state = nil, nil, nil, nil
end)

return R
