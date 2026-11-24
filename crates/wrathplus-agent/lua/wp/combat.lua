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
--   Combat.engaged(guid) for an intent's done: true once that fight is over (and the outcome, and
--                        for "no_progress" what was seen).
--   Combat.defend        (default true) with no fight of its own, engage whatever is attacking you;
--                        and while chasing a target out of reach, turn on one already attacking
--                        you within reach (melee or ranged, each by its own reach).
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
-- "died" | "fled" | "no_progress" [, why]). "no_progress": in reach for `no_progress` seconds, the
-- target's health never dropped, the game refused our actions on it (its red error lines, the
-- rotation's failed casts) and we lost health meanwhile. Combat does not judge what that means
-- (a target we cannot hurt, a client fault): it stops, says what it saw in `why` (and in
-- Combat.last.why, with the others attacking us in Combat.last.attackers), and leaves that unit
-- to the planner. For `no_progress_hold` seconds neither `defend` nor Goals' own pulls turn back
-- to it (Combat.gave_up(guid)); Combat.engage still does, if you choose to.
-- Combat.history keeps the last fights ({ name, outcome, seconds, hp_lost });
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
  -- again when its health has not dropped for `stall` seconds in reach (a melee swing; from range,
  -- facing and targeting it again for the rotation's casts).
  face_within = 0.6, stall = 4,
  -- End a fight as "no_progress" after this many seconds in reach without a scratch on the target,
  -- our actions on it refused and our health falling (false: never); leave that unit to the
  -- planner for `no_progress_hold` seconds.
  no_progress = 12, no_progress_hold = 60,
  given_up = {},
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
-- Since the target's health last dropped (in reach): when, its health, ours, the rotation's failed
-- casts so far, and the game's refusals since ({ [message] = n }).
local still

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
  local n, won, died, fled, stuck, seconds = 0, 0, 0, 0, 0, 0
  local from = math.max(1, #C.history - C.review_every + 1)
  for i = from, #C.history do
    local f = C.history[i]
    n, seconds = n + 1, seconds + f.seconds
    if f.outcome == "killed" then won = won + 1 elseif f.outcome == "died" then died = died + 1
    elseif f.outcome == "fled" then fled = fled + 1 elseif f.outcome == "no_progress" then stuck = stuck + 1 end
  end
  local spells = {}
  for spell, st in pairs(Rotation and Rotation.stats or {}) do
    spells[#spells + 1] = string.format("%s %d cast%s", spell, st.cast, st.failed > 0 and (", " .. st.failed .. " failed") or "")
  end
  table.sort(spells)
  local text = string.format(
    "%s. Last %d fights: %d won, %d died%s%s, %ds on average. Rotation (%s): %s.%s",
    reason, n, won, died, fled > 0 and (", " .. fled .. " fled") or "",
    stuck > 0 and (", " .. stuck .. " left without progress") or "", n > 0 and math.floor(seconds / n) or 0,
    Rotation and Rotation.describe() or "none", #spells > 0 and table.concat(spells, "; ") or "no spells cast",
    weapon_note())
  if Rotation then Rotation.reset_stats() end
  return text
end

-- The goal of a walk a defensive fight broke off, to walk on to after it.
local resume
-- How the last fights ended, by guid (the last ten).
local ended, ended_order = {}, {}

local function done(outcome, why, attackers)
  local guid = C.active
  local seconds = started and math.floor(GetTime() - started) or 0
  local hp_lost = (hp_at_start or 0) - (UnitHealth("player") or 0)
  C.active, C.state, aimed, swung, seen_at, faced, still = nil, nil, nil, nil, nil, nil, nil
  died_at, opened_at, emptied_at, opens = nil, nil, nil, 0
  C.last = { guid = guid, outcome = outcome, why = why, attackers = attackers }
  -- By guid as well: a fight `defend` starts at once (an attacker still on us) clears C.last, and an
  -- intent waiting on this one must still read how it ended.
  if guid then
    if not ended[guid] then ended_order[#ended_order + 1] = guid end
    ended[guid] = C.last
    if #ended_order > 10 then ended[table.remove(ended_order, 1)] = nil end
  end
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
    if why then Agent.signal("COMBAT_DONE", guid, outcome, why) else Agent.signal("COMBAT_DONE", guid, outcome) end
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
  C.last, C.fleeing, hurt_hp, hurt_at, still = nil, nil, nil, nil, nil
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
  local last = guid and ended[guid] or C.last
  return true, last and last.outcome == "killed", last and last.outcome, last and last.why
end

-- Whether a fight with `guid` ended "no_progress" less than `no_progress_hold` seconds ago.
function C.gave_up(guid)
  local til = guid and C.given_up[guid]
  if til and GetTime() >= til then C.given_up[guid], til = nil, nil end
  return til ~= nil
end

-- The living units fighting us within `yards` (not `except`), nearest first.
local function attackers(yards, except)
  local me = UnitGUID("player")
  local out = {}
  for _, u in ipairs(AgentNearby(yards)) do
    if me and u.guid ~= except and not u.dead and u.in_combat and u.target == me and u.reaction ~= "friendly" then
      out[#out + 1] = u
    end
  end
  table.sort(out, function(a, b) return a.distance < b.distance end)
  return out
end

-- The nearest of them, or nil; `fresh` leaves out units a fight was given up on.
local function attacker(yards, except, fresh)
  for _, u in ipairs(attackers(yards, except)) do
    if not (fresh and C.gave_up(u.guid)) then return u end
  end
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

-- Whether we fight this unit with swings: a melee class, or a hunter with the mob on top of it.
local function swings(u)
  return not C.ranged or (u.distance < 5 and select(2, UnitClass("player")) == "HUNTER")
end

-- The rotation's failed casts so far (Rotation.stats), to tell new refusals from old ones.
local function cast_failures()
  local n = 0
  for _, st in pairs(Rotation and Rotation.stats or {}) do n = n + (st.failed or 0) end
  return n
end

-- A fight going nowhere: why, once the target's health has not dropped for `no_progress` seconds
-- in reach while the game refused our actions on it and we lost health; else nil.
local function no_progress(u, now)
  local hp, mine = u.health or 0, UnitHealth("player") or 0
  if not still or hp < still.hp then
    still = { at = now, hp = hp, mine = mine, failed = cast_failures(), refused = {}, n = 0 }
    return nil
  end
  if not C.no_progress or now - still.at < C.no_progress then return nil end
  local lost, failed = still.mine - mine, math.max(0, cast_failures() - still.failed)
  if lost <= 0 or (still.n == 0 and failed == 0) then return nil end
  local parts = {}
  for message, n in pairs(still.refused) do parts[#parts + 1] = string.format('"%s" x%d', message, n) end
  table.sort(parts)
  if failed > 0 then parts[#parts + 1] = failed .. " rotation cast" .. (failed > 1 and "s" or "") .. " failed" end
  return string.format("%s's health has not dropped in %ds in reach (%d) while we lost %d; refused: %s",
    u.name or "the target", math.floor(now - still.at), hp, lost, table.concat(parts, ", "))
end

-- No progress: stop, say what was seen and who else is on us, and leave the unit be a while.
-- Whether to fight one of the others, run or try again is the planner's call, not ours.
local function give_up(why)
  local others, names = {}, {}
  for _, a in ipairs(attackers(40, C.active)) do
    others[#others + 1] = { guid = a.guid, name = a.name, distance = a.distance, health = a.health }
    names[#names + 1] = string.format("%s (%s, %d yd)", tostring(a.name), tostring(a.guid), math.floor(a.distance or 0))
  end
  if #names > 0 then why = why .. "; also attacking us: " .. table.concat(names, ", ") end
  for guid, til in pairs(C.given_up) do
    if GetTime() >= til then C.given_up[guid] = nil end
  end
  C.given_up[C.active] = GetTime() + C.no_progress_hold
  done("no_progress", why, others)
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
  if u.distance > C.reach and C.defend then
    -- Something else is already attacking us within our reach while we chase: fight that one
    -- first (a caster's reach is its range).
    local other = attacker(C.reach + 1, C.active, true)
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
    swung, hurt_hp, still = nil, nil, nil
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
    if swings(u) and (not swung or now - swung >= 1.5) then
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
    hurt_at, faced = now, now
    if AgentFace then AgentFace(C.active) end
    if swings(u) then
      swung = now
      if StartAttack then StartAttack() else AgentInteract(C.active) end
    elseif UnitGUID and UnitGUID("target") ~= C.active then
      -- From range the rotation's casts are the attack: a melee swing would only walk in or be
      -- refused as too far. They need the unit targeted (and faced, above); the rotation runs below.
      AgentTarget(C.active)
    end
    C.kicks = (C.kicks or 0) + 1
  end
  local stuck = no_progress(u, now)
  if stuck then return give_up(stuck) end
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
  -- A refusal while the target's health stands still: evidence for `no_progress` (the global
  -- cooldown and a cast already going are not refusals).
  if still and type(message) == "string" then
    local lower = message:lower()
    if not (lower:find("not ready") or lower:find("another action")) then
      still.refused[message], still.n = (still.refused[message] or 0) + 1, still.n + 1
    end
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
    local foe = attacker(30, nil, true)
    if foe then
      local walk = AgentWalk()
      begin(foe)
      resume = walk and (walk.status == "walking" or walk.status == "routing") and walk.goal or nil
    end
  end
end)

return C
