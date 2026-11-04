-- wp.avoid: keep walks clear of hostile mobs the agent can see, unless the fast tier says to fight
-- through. Loaded for every agent as the global `Avoid`.
--
-- Every second the hostile (red) mobs in sight become circles of their aggro radius (20 yards at
-- the same level, a yard more or less per level of difference, 5 to 45, plus `margin`), which
-- every route then goes round when a fair detour exists (AgentAvoid). The mob Combat is fighting
-- is never avoided. When a walk is under way and mobs not seen before sit on what is left of its
-- route, the fast tier is asked (Agent.decide "pass_hostiles"): "around" re-routes the walk now,
-- "fight" leaves those mobs out of the circles for `fight_for` seconds, so the walk goes through
-- and Combat (Combat.defend) takes them on as they come. With no answer in time, it goes round.
--
--   Avoid.on          false: no circles, no questions (the agent wants every fight).
--   Avoid.zones       the circles in use, { { x, y, r, guid, name, level }, ... }.
--   Avoid.fight(guid) leave a mob out for `fight_for` seconds (the planner choosing a fight).
--   Avoid.last        the last question and its answer, for the status line.

local M = { on = true, margin = 3, range = 90, fight_for = 120, zones = {}, last = nil }

local fighting = {} -- guid -> until when it is left out
local asked = {}    -- guid -> true once the fast tier has been asked about it
local decided = nil -- the open decision's id

local function radius(level)
  local mine = UnitLevel("player") or 1
  local r = 20 + ((level or mine) - mine)
  return math.max(5, math.min(45, r)) + M.margin
end

-- The hostile, living, not-yet-fought mobs in sight, as circles.
local function circles(now)
  local out = {}
  for _, u in ipairs(AgentNearby(M.range)) do
    if u.kind == "unit" and u.reaction == "hostile" and not u.dead and u.x
      and u.guid ~= (Combat and Combat.active) and (fighting[u.guid] or 0) < now then
      out[#out + 1] = { x = u.x, y = u.y, r = radius(u.level), guid = u.guid, name = u.name, level = u.level }
    end
  end
  return out
end

-- Whether the circle sits on the walk still ahead (from where we stand through its corners).
local function on_route(z, route, me)
  local px, py = me.x, me.y
  for _, p in ipairs(route) do
    local dx, dy = p[1] - px, p[2] - py
    local len2 = dx * dx + dy * dy
    local t = len2 > 0 and math.max(0, math.min(1, ((z.x - px) * dx + (z.y - py) * dy) / len2)) or 0
    local cx, cy = px + dx * t, py + dy * t
    if (z.x - cx) ^ 2 + (z.y - cy) ^ 2 < z.r * z.r then return true end
    px, py = p[1], p[2]
  end
  return false
end

-- Walk on to the same goal (or interaction) over a route that now goes round.
local function reroute(walk)
  if walk.interact then
    AgentInteract(walk.interact)
  elseif walk.goal then
    AgentMoveTo(walk.goal[1], walk.goal[2], walk.goal[3])
  end
end

local function ask(threats, walk)
  local names, guids = {}, {}
  for _, z in ipairs(threats) do
    names[#names + 1] = string.format("%s (level %s)", tostring(z.name), tostring(z.level))
    guids[#guids + 1] = z.guid
    asked[z.guid] = true
  end
  local question = string.format("Your walk passes %d hostile mob%s: %s. Go round them, or fight through?",
    #threats, #threats == 1 and "" or "s", table.concat(names, ", "))
  decided = Agent.decide("pass_hostiles", {
    question = question,
    options = {
      around = "Go round them: a longer walk, no fight",
      fight = "Walk through and fight them (worth it for a quest, loot or XP, and safe at this health)",
    },
    context = {
      health = math.floor(100 * (UnitHealth("player") or 0) / math.max(1, UnitHealthMax("player") or 1)),
      level = UnitLevel("player"),
      mobs = table.concat(names, ", "),
    },
    default = "around",
    timeout = 3,
    on = function(choice)
      decided = nil
      M.last = { question = question, choice = choice }
      if choice == "fight" then
        for _, g in ipairs(guids) do M.fight(g) end
        return
      end
      local now = AgentWalk()
      if now.status == "walking" then reroute(now) end
    end,
  })
end

function M.fight(guid)
  fighting[guid] = GetTime() + M.fight_for
end

local function tick(now)
  if not M.on then
    if #M.zones > 0 then M.zones = {} AgentAvoid({}) end
    return
  end
  local zones = circles(now)
  M.zones = zones
  AgentAvoid(zones)
  if decided or not (Agent and Agent.decide) then return end
  local walk = AgentWalk()
  local me = AgentPosition()
  if walk.status ~= "walking" or not walk.route or not me then return end
  local threats = {}
  for _, z in ipairs(zones) do
    if not asked[z.guid] and on_route(z, walk.route, me) then threats[#threats + 1] = z end
  end
  if #threats > 0 then ask(threats, walk) end
end

local frame = CreateFrame("Frame")
local since = 0
frame:SetScript("OnUpdate", function(_, dt)
  since = since + dt
  if since < 1 then return end
  since = 0
  if UnitIsDeadOrGhost("player") then return end
  tick(GetTime())
end)

return M
