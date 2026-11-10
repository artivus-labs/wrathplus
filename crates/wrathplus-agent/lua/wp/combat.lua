-- wp.combat: fighting a target, kept in range. Loaded for every agent as the global `Combat`.
--
--   Combat.engage(guid [, { force = true }])
--                        target the unit, chase it into range through the navmesh (routing again
--                        as it moves), keep facing it, auto-attack and cast the Rotation
--                        (wp.rotation, from the approach on, so Charge opens), until it dies, is
--                        lost, or Combat.stop() is called. Returns true if the fight started.
--                        A pull Survive.pull_ok calls unsafe (too high a level, an elite, more
--                        hostiles in joining distance, our health or a broken weapon, a spot we
--                        just died at) is refused: false and why (also Combat.refused, and
--                        AGENT_PULL_REFUSED). `force` pulls anyway; `Combat.safe = false` never
--                        refuses. Casters and hunters fight from `reach` yards (their class's
--                        range), facing the target without walking in.
--                        Returns at once: the fight runs in the background and ends as
--                        AGENT_COMBAT_DONE. Out of sight is false and why: during a cinematic
--                        (a new character's intro starts a few seconds after it enters the world,
--                        hiding every unit) the reason says so, and a running fight waits it out
--                        instead of counting its target lost.
--   Combat.active        the guid being fought, or nil. Combat.state says what it is doing.
--   Combat.engaged(guid) for an intent's done: true once that fight is over (and the outcome).
--   Combat.defend        (default true) with no fight of its own, engage whatever is attacking you;
--                        and while chasing a target out of reach, turn on one already hitting you.
--                        A walk such a fight broke off is walked on once it is won. Defence asks
--                        nobody: no pull check, and a wrapper round Combat.engage cannot stop it.
--   Combat.flee([why])   leave the fight and run: snare the mob if the class can, then run to
--                        Survive.escape_point() (away from the attackers, clear of other
--                        hostiles), leg after leg until nothing is on us, without `defend`
--                        dragging us back. Combat.fleeing says so meanwhile. Survive calls it
--                        when a fight is lost and the run can be survived; after `flee_for`
--                        seconds with a mob still at our heels it turns and fights it out.
--                        Running with Combat.defend = false and a walk of your own leaves you
--                        hit in the back all the way: use this instead.
--   Combat.loot          (default true) loot a kill as a player would, money and every item,
--                        before the fight counts as over: walk to the corpse, open it, take all.
--                        A corpse that is not ours to loot (another's tap, nothing on it) is left
--                        once `loot_wait` seconds pass without it turning lootable. False leaves
--                        corpses to your own code.
--
-- While it fights, Survive (wp.survive) watches both sides' health: it drinks a healing potion or
-- healthstone, presses the class's emergency buttons, and says when to flee.
--
-- The end of a fight is signalled as AGENT_COMBAT_DONE (guid, "killed" | "lost" | "stopped" |
-- "died" | "fled"). Combat.history keeps the last fights ({ name, outcome, seconds, hp_lost });
-- every `review_every` fights, and after a death, AGENT_COMBAT_REVIEW carries a summary with the
-- rotation's casts and failures, for the planner to rework Rotation.list.

local C = {
  active = nil,
  state = nil,
  -- Melee range in yards, below the server's (5 plus both units' combat reach).
  reach = 3,
  -- Route again once the target has moved this far from where the walk was aimed.
  drift = 2.5,
  -- Give up on a unit out of sight this long; `sight` is how far a unit can be engaged from
  -- (what Goals reads the world to).
  lose_after = 5, sight = 100,
  last = nil,
  history = {},
  fights = 0,
  review_every = 10,
  -- Set reach from the class on each fight (casters and hunters keep their distance); false keeps
  -- a reach of your own.
  auto_reach = true,
  defend = true,
  loot = true,
  -- How long a fresh corpse may take to turn lootable (the flag follows the death).
  loot_wait = 1.5,
  -- Refuse pulls Survive calls unsafe (Combat.engage's `force` overrides one).
  safe = true,
  -- Turn to the target when it stands more than this (radians) off our facing; start the attack
  -- again when its health has not dropped for `stall` seconds in reach.
  face_within = 0.6, stall = 4,
  -- Give up running after this long with a mob still on us, and fight it out.
  flee_for = 20,
  fleeing = nil,
  refused = nil,
}

local name_of, started, hp_at_start, faced, target_health
-- Looting a kill: when it died, and when the corpse was last opened.
local died_at, opened_at, emptied_at
local opens = 0

local aimed, swung, seen_at
-- The target's health when it last dropped, and when: a fight in reach where it stops dropping
-- means our attack is not going out.
local hurt_hp, hurt_at

local function find(guid)
  for _, u in ipairs(AgentNearby(C.sight)) do
    if u.guid == guid then return u end
  end
end

-- Why nothing is in sight or walkable right now (a cinematic), else nil.
local function held()
  return AgentCinematic and AgentCinematic() or nil
end

local function far(a, b)
  return math.sqrt((a.x - b.x) ^ 2 + (a.y - b.y) ^ 2)
end

-- What the planner should know about the weapon: one far below our level makes every fight long.
local function weapon_note()
  if not (GetInventoryItemLink and GetItemInfo) then return "" end
  local link = GetInventoryItemLink("player", 16)
  if not link then return " No weapon in the main hand." end
  local name, _, _, ilevel = GetItemInfo(link)
  local level = UnitLevel("player") or 1
  if name and ilevel and ilevel + 5 < level then
    return string.format(" Your weapon (%s, item level %d) is far below level %d: a better one shortens every fight.",
      name, ilevel, level)
  end
  return ""
end

-- A summary of the last fights and the rotation's record over them, for the planner.
local function review(reason)
  local n, won, died, fled, seconds = 0, 0, 0, 0, 0
  local from = math.max(1, #C.history - C.review_every + 1)
  for i = from, #C.history do
    local f = C.history[i]
    n, seconds = n + 1, seconds + f.seconds
    if f.outcome == "killed" then won = won + 1 elseif f.outcome == "died" then died = died + 1
    elseif f.outcome == "fled" then fled = fled + 1 end
  end
  local spells = {}
  for spell, st in pairs(Rotation and Rotation.stats or {}) do
    spells[#spells + 1] = string.format("%s %d cast%s", spell, st.cast, st.failed > 0 and (", " .. st.failed .. " failed") or "")
  end
  table.sort(spells)
  local text = string.format(
    "%s. Last %d fights: %d won, %d died%s, %ds on average. Rotation (%s): %s.%s",
    reason, n, won, died, fled > 0 and (", " .. fled .. " fled") or "", n > 0 and math.floor(seconds / n) or 0,
    Rotation and Rotation.describe() or "none", #spells > 0 and table.concat(spells, "; ") or "no spells cast",
    weapon_note())
  if Rotation then Rotation.reset_stats() end
  return text
end

-- The goal of a walk a defensive fight broke off, to walk on to after it.
local resume

local function done(outcome)
  local guid = C.active
  local seconds = started and math.floor(GetTime() - started) or 0
  local hp_lost = (hp_at_start or 0) - (UnitHealth("player") or 0)
  C.active, C.state, aimed, swung, seen_at, faced = nil, nil, nil, nil, nil, nil
  died_at, opened_at, emptied_at, opens = nil, nil, nil, 0
  C.last = { guid = guid, outcome = outcome }
  if outcome ~= "stopped" then
    C.history[#C.history + 1] = { name = name_of, outcome = outcome, seconds = seconds, hp_lost = hp_lost }
    if #C.history > 20 then table.remove(C.history, 1) end
    C.fights = C.fights + 1
  end
  if outcome == "killed" and Survive then Survive.fight_won(target_health, seconds, name_of) end
  if AgentStop then AgentStop() end
  if resume and (outcome == "killed" or outcome == "lost") then
    AgentMoveTo(resume[1], resume[2], resume[3])
  end
  resume = nil
  if Agent and Agent.signal then
    Agent.signal("COMBAT_DONE", guid, outcome)
    if outcome == "died" then
      Agent.signal("COMBAT_REVIEW", review("You died fighting " .. tostring(name_of)))
    elseif outcome ~= "stopped" and C.fights % C.review_every == 0 then
      Agent.signal("COMBAT_REVIEW", review("Fight review"))
    end
  end
end

-- Start the fight with a unit in sight. Defence comes straight here: no pull check.
local function begin(u)
  C.active, aimed, swung, seen_at, faced = u.guid, nil, nil, GetTime(), nil
  C.last, C.fleeing, hurt_hp, hurt_at = nil, nil, nil, nil
  name_of, started, hp_at_start, target_health = u.name, GetTime(), UnitHealth("player"), u.health_max
  if C.auto_reach then
    local _, class = UnitClass("player")
    local range = Rotation and Rotation.RANGE and Rotation.RANGE[class or ""]
    C.reach, C.ranged = range or 3, range ~= nil
  end
  if Survive then
    Survive.stand()
    Survive.fight_start()
  end
  AgentTarget(u.guid)
  if C.ranged and PetAttack and UnitExists and UnitExists("pet") then PetAttack() end
  return true
end

local refused_at = -60

function C.engage(guid, opts)
  local u = find(guid)
  if not u then return false, held() or "not in sight" end
  if C.safe and Survive and not (opts and opts.force) then
    local ok, why = Survive.pull_ok(u)
    if not ok then
      C.refused = { guid = guid, name = u.name, why = why, at = GetTime() }
      -- Said once a minute: a module that keeps asking does not flood the planner.
      if Agent and Agent.signal and GetTime() - refused_at >= 60 then
        refused_at = GetTime()
        Agent.signal("PULL_REFUSED", tostring(u.name), tostring(why))
      end
      return false, why
    end
  end
  return begin(u)
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

-- The nearest living unit fighting us within `yards` (not `except`), or nil.
local function attacker(yards, except)
  local me = UnitGUID("player")
  local best
  for _, u in ipairs(AgentNearby(yards)) do
    if me and u.guid ~= except and not u.dead and u.in_combat and u.target == me and u.reaction ~= "friendly"
      and (not best or u.distance < best.distance) then
      best = u
    end
  end
  return best
end

---------------------------------------------------------------------------------------------------
-- Running away

local function run_leg(now)
  local foes = Survive and Survive.attackers(nil) or {}
  local p = Survive and Survive.escape_point(foes)
  if p then AgentMoveTo(p.x, p.y, p.z) end
  C.fleeing.leg = now
end

function C.flee(why)
  local now = GetTime()
  local from = C.active and name_of or "a fight"
  if C.active then done("fled") end
  C.fleeing = { since = now, why = why or "fleeing" }
  if Survive then Survive.scare("fled " .. tostring(from)) end
  -- No more swings at what we turn our back on (each one is a red error line).
  if StopAttack then StopAttack() end
  -- Slow it first, so the run gets clear (it needs the mob still targeted and in reach).
  local snare = Survive and Survive.snare()
  if snare and Rotation.cast(snare) then C.fleeing.snare = snare end
  run_leg(now)
  C.state = "fleeing" .. (C.fleeing.snare and (" (" .. C.fleeing.snare .. ")") or "")
  if Agent and Agent.signal then Agent.signal("FLEEING", tostring(why or "the fight is lost")) end
  return true
end

local function flee_tick(now)
  local f = C.fleeing
  local foe = attacker(40)
  if not foe and not UnitAffectingCombat("player") then
    C.fleeing, C.state = nil, nil
    if AgentStop then AgentStop() end
    if Agent and Agent.signal then Agent.signal("FLED", math.floor(now - f.since)) end
    return
  end
  if foe and foe.distance < 8 and now - f.since >= C.flee_for then
    -- Still at our heels: running has not worked, so turn and fight with what is left.
    C.fleeing, C.state = nil, nil
    if Survive then Survive.cornered() end
    return
  end
  if Survive and Survive.hp() < Survive.critical then Survive.heal_item() end
  if not f.snare and foe and foe.distance < 5 then
    local snare = Survive and Survive.snare()
    if snare and Rotation.cast(snare) then f.snare = snare end
  end
  if AgentWalk().status ~= "walking" and now - (f.leg or 0) >= 1 then run_leg(now) end
end

---------------------------------------------------------------------------------------------------
-- The fight

-- The kill is down: loot it (walk over, open it; LOOT_OPENED below takes everything), then end.
local function loot_kill(u, now)
  died_at = died_at or now
  if not C.loot or attacker(30) then return done("killed") end
  if not u.lootable then
    -- Opened and emptied, or never ours: done once the flag has had time to arrive.
    if opened_at or now - died_at >= C.loot_wait then return done("killed") end
    C.state = "waiting to loot " .. (u.name or "the kill")
    return
  end
  if u.distance > C.reach then
    if AgentWalk().status ~= "walking" then AgentMoveTo(u.x, u.y, u.z) end
    C.state = "walking to loot " .. (u.name or "the kill")
    return
  end
  -- Still lootable after three opens: something on it we cannot take (bags full, a quest item we
  -- do not need). Leave it rather than open it forever.
  if opens >= 3 then return done("killed") end
  if LootFrame and LootFrame:IsShown() then
    -- The picks went out when it opened; they land on a later frame. Close only after that.
    if emptied_at and now - emptied_at >= 1 then CloseLoot() end
  elseif not opened_at or now - opened_at >= 2 then
    opened_at, opens = now, opens + 1
    AgentInteract(C.active)
  end
  C.state = "looting " .. (u.name or "the kill")
end

-- Whether the unit stands outside our front arc (a swing or a cast at it would be refused).
local function turned_away(u)
  local me = AgentPosition and AgentPosition()
  if not (me and me.facing and u.x) then return false end
  local want = math.atan2(u.y - me.y, u.x - me.x)
  local off = math.abs((want - me.facing + math.pi) % (2 * math.pi) - math.pi)
  return off > C.face_within
end

-- The rotation, and what it cast. A rotation's bug must not stop the fight: say what broke.
local function rotate(u)
  if not Rotation then return nil end
  local ok, cast = pcall(Rotation.tick, u, started)
  if ok then return cast end
  C.rotation_error = tostring(cast)
end

local function tick(now)
  if UnitIsDeadOrGhost("player") then return done("died") end
  -- A cinematic: the target is out of sight with everything else, and no walk would start. That
  -- is not the target lost: wait it out (the agent may skip it; we never do).
  local why = held()
  if why then
    seen_at = now
    C.state = "waiting: " .. why
    return
  end
  local u = find(C.active)
  if not u then
    if now - (seen_at or now) >= C.lose_after then return done("lost") end
    C.state = "looking for the target"
    return
  end
  seen_at = now
  if u.dead then return loot_kill(u, now) end
  -- Survival first: a potion, an emergency button, or the call to run.
  local saved
  if Survive then
    local ok, what = pcall(Survive.fight_tick, u)
    if ok and what == "flee" then return C.flee("losing to " .. (u.name or "the target")) end
    saved = ok and what or nil
    if not ok then C.survive_error = tostring(what) end
  end
  if u.distance > C.reach and C.defend and not C.ranged then
    -- Something else is already hitting us while we chase: fight that one first.
    local other = attacker(C.reach + 1, C.active)
    if other then
      local walk_on = resume
      done("stopped")
      begin(other)
      resume = walk_on
      return
    end
  end
  if u.distance > C.reach then
    -- Chase: aim at where it stands, and again whenever it has moved on.
    if not aimed or far(aimed, u) > C.drift or AgentWalk().status ~= "walking" then
      AgentMoveTo(u.x, u.y, u.z)
      aimed = { x = u.x, y = u.y }
    end
    swung, hurt_hp = nil, nil
    -- An opener on the way in (Charge has a range band; melee spells wait for reach).
    local cast = u.distance <= 30 and rotate(u) or nil
    C.state = string.format("closing on %s (%d yd)%s", u.name or "the target", u.distance, cast and (": " .. cast) or "")
    return
  end
  -- In reach: stop the chase, a route still coming from the realm included.
  local chase = AgentWalk().status
  if chase == "walking" or chase == "routing" then AgentStop() end
  aimed = nil
  if C.ranged then
    -- From range: turn to it (a cast needs facing), without the attack's walk into melee.
    if not faced or now - faced >= 1 then
      faced = now
      if AgentFace then AgentFace(C.active) end
    end
    -- A hunter with the mob on top of it swings back.
    if u.distance < 5 and select(2, UnitClass("player")) == "HUNTER" and (not swung or now - swung >= 1.5) then
      swung = now
      AgentInteract(C.active)
    end
  else
    -- In melee: the right-click attack faces the unit and starts auto-attack; between swings,
    -- turn again as soon as it steps out of our front arc (mobs shuffle round us).
    if not swung or now - swung >= 1.5 then
      swung, faced = now, now
      AgentInteract(C.active)
    elseif AgentFace and turned_away(u) and (not faced or now - faced >= 0.4) then
      faced = now
      AgentFace(C.active)
    end
  end
  -- In reach and its health has not moved for `stall` seconds: start the attack again, facing it.
  if not hurt_hp or (u.health or 0) < hurt_hp then
    hurt_hp, hurt_at = u.health or 0, now
  elseif now - hurt_at >= C.stall then
    hurt_at, swung, faced = now, now, now
    if AgentFace then AgentFace(C.active) end
    if StartAttack then StartAttack() else AgentInteract(C.active) end
    C.kicks = (C.kicks or 0) + 1
  end
  local cast = rotate(u) or saved
  C.state = "fighting " .. (u.name or "the target") .. (cast and (": " .. cast) or "")
end

local frame = CreateFrame("Frame")
local since = 0
frame:RegisterEvent("UI_ERROR_MESSAGE")
frame:RegisterEvent("LOOT_OPENED")
frame:RegisterEvent("LOOT_BIND_CONFIRM")
frame:SetScript("OnEvent", function(_, event, message)
  -- Our kill's corpse is open: take everything, as autoloot would.
  if event == "LOOT_OPENED" then
    if C.active and died_at then
      -- Picks are queued for a later frame: closing now would release the corpse before they
      -- land. loot_kill closes it a second later if it is still open.
      for slot = GetNumLootItems(), 1, -1 do LootSlot(slot) end
      emptied_at = GetTime()
    end
    return
  end
  -- A bind-on-pickup item from our own kill is still ours to take.
  if event == "LOOT_BIND_CONFIRM" then
    if C.active and died_at then ConfirmLootSlot(message) end
    return
  end
  -- The server says too far or facing away: take it as a cue to close in and face again.
  if C.active and type(message) == "string"
    and (message:find("too far") or message:find("in front") or message:find("facing")) then
    aimed, swung, faced = nil, nil, nil
  end
end)
frame:SetScript("OnUpdate", function(_, dt)
  since = since + dt
  if since < 0.2 then return end
  since = 0
  if C.fleeing then
    if UnitIsDeadOrGhost("player") then
      C.fleeing, C.state = nil, nil
      if Agent and Agent.signal then
        Agent.signal("COMBAT_REVIEW", review("You died running from " .. tostring(name_of)))
      end
    else
      flee_tick(GetTime())
    end
  elseif C.active then
    tick(GetTime())
  elseif C.defend and not UnitIsDeadOrGhost("player") and not held() then
    -- Attacked with no fight of our own (resting, walking, between pulls): fight back.
    local foe = attacker(30)
    if foe then
      local walk = AgentWalk()
      begin(foe)
      resume = walk and (walk.status == "walking" or walk.status == "routing") and walk.goal or nil
    end
  end
end)

return C
