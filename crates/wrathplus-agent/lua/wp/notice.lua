-- wp.notice: what the character passes, so the planner perceives it. Loaded for every agent as the
-- global `Notice`. A goal runs the character straight at its target, and the planner only hears
-- about the world when something wakes it: the quest giver, trainer or flight master on the way
-- goes unseen. This looks around every pass and signals what is new to this character; what to
-- make of it (detour, note it, ignore it) is the planner's call, or a Lua reflex of its own.
--
-- Every `every` seconds (and whenever Goals reads the world for its facts: that scan is shared, and
-- a pass after one does not read the world again), the points of interest within `radius` yards
-- are compared with what this character has already noticed. Each new one is signalled
--   AGENT_NOTICED(kind, name, guid, distance, x, y, extra)
-- and kept in Notice.recent (newest last, `recent_max`), with the full record
--   { kind, name, guid, entry, distance, x, y, z, extra, title, level, trivial, zone, subzone,
--     muted, handled, at, seq }.
-- Kinds, from AgentNearby (the client's own reading) and GetSubZoneText:
--   quest_available  a giver with a "!" for us (quest "available"), or a grey "!" (quest
--                    "available_low": only quests far below us): trivial = true, extra "trivial".
--                    The client does not say the quest's level before the giver is spoken to.
--   quest_turnin     a giver with a "?" for us (quest "reward").
--   trainer          NPC service trainer; extra "class" or "profession" (and the title).
--   vendor, repair, innkeeper, banker, auctioneer, mailbox   NPC services; extra is the title.
--   flightmaster     extra "unknown" when AgentNearby says its node is not learnt (taxi field),
--                    "known" when it says it is.
--   rare, elite      a unit not friendly to us, ranked rare/rareelite or elite/worldboss; extra its
--                    level and rank.
--   node             a herb or a vein (a game object whose lock shows GatherHerbs or Mine); extra
--                    "herb" or "ore".
--   chest            a locked chest (PickLock) or a chest, trunk, strongbox, coffer or footlocker
--                    by name; extra "locked" when locked.
--   mailbox          also a mailbox game object.
--   player           another player in sight (bots are players too: the client cannot tell);
--                    extra its level.
--   subzone          a subzone entered for the first time (name, distance 0, where we stand);
--                    extra the zone.
-- An NPC offering several services is one notice per kind (one guid): mute one kind, not the NPC.
-- Novelty is per character: units and players by guid, game objects by entry and place, subzones
-- by zone and name. It lasts the session (the Lua VM): a relog or a client restart notices the
-- surroundings again. Notice.forget([kind]) forgets now.
--
-- Mutes: Notice.mute(kind [, filter]) stops signalling that kind (all of it, or the notices for
--   which filter(n) is true; filter is a function or Lua source over `n`, as the planner tools send
--   it, e.g. Notice.mute("quest_available", "n.trivial")). A muted notice is still recorded
--   (Notice.recent, Notice.passed): only the signal is held back. Mutes add up; Notice.unmute(kind)
--   lifts them all, Notice.mutes() lists them. A human with a goal ignores what it cannot use:
--   grey quests, other players, herbs without Herbalism.
-- Notice.passed([since]) → { next, dropped, kinds = { { kind, count, muted, names }, ... } }: what
--   was noticed after the cursor `since` (0 or nil: all kept), counted by kind, most first, with
--   up to `digest_names` names each; pass `next` as `since` the next time. `dropped` counts those
--   after the cursor that fell out of the log (`log_max`) before the read. The launcher's
--   check-ins read it, so reflection sees what was passed, muted or not.
-- Reflexes: Notice.listen(name, fn) calls fn(n) for each new notice, muted or not; returning true
--   takes it in hand, and it is not signalled (it is still recorded). A listener can act without
--   waking the planner (a detour to a "!" in reach), if the agent wants one. Notice.unlisten(name).
-- Notice.on = false stops looking.

local N = {
  on = true, radius = 60, every = 1,
  -- Noticed this session: key -> { kind, name, at }.
  seen = {}, seen_count = 0, seen_max = 5000,
  recent = {}, recent_max = 50,
  -- Every notice in order for Notice.passed: { seq, kind, name, muted }.
  log = {}, log_max = 1000, seq = 0,
  digest_names = 6,
  muted_by = {}, listeners = {}, listen_errors = {},
}

local KINDS = { "quest_available", "quest_turnin", "trainer", "vendor", "repair", "flightmaster",
  "innkeeper", "banker", "mailbox", "auctioneer", "rare", "elite", "node", "chest", "player",
  "subzone" }
N.kinds = KINDS
local KNOWN = {}
for _, k in ipairs(KINDS) do KNOWN[k] = true end

-- NPC services (AgentNearby's npc) noticed as themselves.
local SERVICE = { vendor = "vendor", repair = "repair", flightmaster = "flightmaster",
  innkeeper = "innkeeper", banker = "banker", auctioneer = "auctioneer", mailbox = "mailbox" }
local CHEST_WORDS = { "chest", "trunk", "strongbox", "coffer", "footlocker" }
-- Game object types (GameObjectTypes): a chest (also every herb and vein), a mailbox.
local GO_CHEST, GO_MAILBOX = 3, 19

local function now() return GetTime() end

local function signal(name, ...)
  if Agent and Agent.signal then Agent.signal(name, ...) end
end

-- A filter from Lua source as the planner sends it: an expression over `n`, else a chunk.
local function compile(code)
  if type(code) ~= "string" then return code end
  local fn = loadstring("local n = ... return " .. code, "=notice filter")
  if not fn then
    local err
    fn, err = loadstring("local n = ... " .. code, "=notice filter")
    assert(fn, "notice filter: " .. tostring(err))
  end
  return fn
end

local function has(list, value)
  for _, v in ipairs(list or {}) do
    if v == value then return true end
  end
  return false
end

local function chest_name(name)
  name = (name or ""):lower()
  for _, w in ipairs(CHEST_WORDS) do
    if name:find(w, 1, true) then return true end
  end
  return false
end

-- The notices an object in sight makes: { kind, extra, key } each.
local function kinds_of(u)
  local out = {}
  local function add(kind, extra, key) out[#out + 1] = { kind = kind, extra = extra, key = key } end
  if u.kind == "unit" and not u.dead then
    local key = "unit:" .. tostring(u.guid)
    if u.quest == "available" then add("quest_available", nil, key)
    elseif u.quest == "available_low" then add("quest_available", "trivial", key)
    elseif u.quest == "reward" then add("quest_turnin", nil, key) end
    local npc = u.npc or {}
    if has(npc, "trainer") or has(npc, "classtrainer") or has(npc, "professiontrainer") then
      local what = has(npc, "classtrainer") and "class" or has(npc, "professiontrainer") and "profession" or nil
      add("trainer", what and u.title and (what .. ": " .. u.title) or what or u.title, key)
    end
    for _, s in ipairs(npc) do
      local kind = SERVICE[s]
      if kind == "flightmaster" then
        add(kind, (u.taxi == "unknown" or u.taxi == "known") and u.taxi or nil, key)
      elseif kind then
        add(kind, u.title, key)
      end
    end
    local rank = u.classification
    local kind = (rank == "rare" or rank == "rareelite") and "rare"
      or (rank == "elite" or rank == "worldboss") and "elite" or nil
    -- A town's guards are elite too: only those not friendly to us are worth a look.
    if kind and u.reaction ~= "friendly" then
      add(kind, string.format("level %s %s", tostring(u.level or "?"), rank), key)
    end
  elseif u.kind == "player" and not u.dead then
    add("player", u.level and ("level " .. u.level) or nil, "player:" .. tostring(u.guid))
  elseif u.kind == "gameobject" and u.x then
    -- A spawn's place, not its guid: a node that respawns where it stood is the same node.
    local key = string.format("go:%s:%d:%d", tostring(u.entry or u.name), math.floor(u.x / 5), math.floor(u.y / 5))
    if u.go_type == GO_MAILBOX then
      add("mailbox", nil, key)
    elseif u.go_type == GO_CHEST then
      if u.lock == "GatherHerbs" then add("node", "herb", key)
      elseif u.lock == "Mine" then add("node", "ore", key)
      elseif u.lock == "PickLock" then add("chest", "locked", key)
      elseif chest_name(u.name) then add("chest", nil, key) end
    end
  end
  return out
end

local function muted(n)
  local filters = N.muted_by[n.kind]
  if not filters then return false end
  for _, f in ipairs(filters) do
    if f == true then return true end
    local ok, yes = pcall(f, n)
    if ok and yes then return true end
  end
  return false
end

-- Whether a listener took the notice in hand (returned true). Listeners hear muted notices too.
local function heard(n)
  local handled = false
  for name, fn in pairs(N.listeners) do
    local ok, yes = pcall(fn, n)
    if not ok then
      N.listen_errors[name] = tostring(yes)
    elseif yes == true then
      handled = true
    end
  end
  return handled
end

local function remember(key, n)
  if not N.seen[key] then N.seen_count = N.seen_count + 1 end
  N.seen[key] = { kind = n.kind, name = n.name, at = n.at }
  if N.seen_count <= N.seen_max then return end
  -- Too many: forget the older half (other players and nodes, mostly, long walked past).
  local ats = {}
  for _, s in pairs(N.seen) do ats[#ats + 1] = s.at end
  table.sort(ats)
  local cut = ats[math.floor(#ats / 2)] or 0
  for k, s in pairs(N.seen) do
    if s.at < cut then N.seen[k], N.seen_count = nil, N.seen_count - 1 end
  end
end

local function record(key, n)
  remember(key, n)
  N.seq = N.seq + 1
  n.seq = N.seq
  n.muted = muted(n)
  n.handled = heard(n)
  N.recent[#N.recent + 1] = n
  while #N.recent > N.recent_max do table.remove(N.recent, 1) end
  N.log[#N.log + 1] = { seq = N.seq, kind = n.kind, name = n.name, muted = n.muted }
  while #N.log > N.log_max do table.remove(N.log, 1) end
  if not n.muted and not n.handled then
    signal("NOTICED", n.kind, n.name, n.guid, n.distance, n.x, n.y, n.extra)
  end
end

local function zone_names()
  local zone = GetRealZoneText and GetRealZoneText() or nil
  local sub = GetSubZoneText and GetSubZoneText() or nil
  return zone ~= "" and zone or nil, sub ~= "" and sub or nil
end

-- Look at a list of objects in sight (AgentNearby's), nearest first, and notice what is new.
local function look(list)
  local zone, sub = zone_names()
  for _, u in ipairs(list) do
    if (u.distance or 0) > N.radius then break end
    for _, k in ipairs(kinds_of(u)) do
      local key = k.kind .. "|" .. k.key
      if not N.seen[key] then
        record(key, { kind = k.kind, name = u.name or "?", guid = u.guid, entry = u.entry,
          distance = u.distance, x = u.x, y = u.y, z = u.z, extra = k.extra, title = u.title,
          level = u.level, trivial = k.extra == "trivial" or nil, zone = zone, subzone = sub, at = now() })
      end
    end
  end
end

local function look_at_subzone()
  local zone, sub = zone_names()
  if not sub then return end
  local key = "subzone|" .. tostring(zone) .. "/" .. sub
  if N.seen[key] then return end
  local pos = AgentPosition and AgentPosition()
  record(key, { kind = "subzone", name = sub, distance = 0, x = pos and pos.x, y = pos and pos.y,
    z = pos and pos.z, extra = zone, zone = zone, subzone = sub, at = now() })
end

-- Goals handed its scan in since the last pass: no need to read the world again.
local fed = false

-- Goals' facts hand their scan of `yards` here, so one read of the world serves both.
function N.look(list, yards)
  if not N.on or type(list) ~= "table" or (yards or 0) < N.radius then return end
  -- A cinematic shows the world around its camera, not around us.
  if AgentCinematic and AgentCinematic() then return end
  fed = true
  look(list)
end

local function pass()
  if not N.on then return end
  look_at_subzone()
  if fed then
    fed = false
    return
  end
  if not AgentNearby then return end
  local list, note = AgentNearby(N.radius)
  -- A cinematic shows the world around its camera, not around us.
  if note then return end
  look(list or {})
end

function N.mute(kind, filter)
  assert(KNOWN[kind], "Notice.mute: kind is one of " .. table.concat(KINDS, ", "))
  if filter == nil then
    N.muted_by[kind] = { true }
    return
  end
  local fn = compile(filter)
  assert(type(fn) == "function", "Notice.mute: filter is a function of the notice, or Lua source over n")
  local list = N.muted_by[kind] or {}
  list[#list + 1] = fn
  N.muted_by[kind] = list
end

function N.unmute(kind)
  assert(KNOWN[kind], "Notice.unmute: kind is one of " .. table.concat(KINDS, ", "))
  N.muted_by[kind] = nil
end

-- { kind = "all" | count of filters }.
function N.mutes()
  local out = {}
  for kind, filters in pairs(N.muted_by) do
    out[kind] = filters[1] == true and "all" or #filters
  end
  return out
end

function N.forget(kind)
  for key, s in pairs(N.seen) do
    if kind == nil or s.kind == kind then N.seen[key], N.seen_count = nil, N.seen_count - 1 end
  end
end

function N.passed(since)
  since = tonumber(since) or 0
  -- A cursor past the end is from before a restart of the client: everything kept is new.
  if since > N.seq then since = 0 end
  local by, order = {}, {}
  for _, e in ipairs(N.log) do
    if e.seq > since then
      local k = by[e.kind]
      if not k then
        k = { kind = e.kind, count = 0, muted = 0, names = {}, named = {} }
        by[e.kind], order[#order + 1] = k, k
      end
      k.count = k.count + 1
      if e.muted then k.muted = k.muted + 1 end
      if not k.named[e.name] and #k.names < N.digest_names then
        k.named[e.name], k.names[#k.names + 1] = true, e.name
      end
    end
  end
  for _, k in ipairs(order) do k.named = nil end
  table.sort(order, function(a, b)
    if a.count ~= b.count then return a.count > b.count end
    return a.kind < b.kind
  end)
  -- Notices after the cursor that fell out of the log before this read.
  local first = N.log[1] and N.log[1].seq or N.seq + 1
  return { next = N.seq, dropped = math.max(0, first - 1 - since), kinds = order }
end

-- A Lua reflex: fn(n) hears every new notice (muted ones too) as it is made; one that returns
-- true has taken it in hand, and it is not signalled. Listening again by a name replaces it;
-- Notice.unlisten(name) drops it. A listener's error is kept in Notice.listen_errors[name].
function N.listen(name, fn)
  assert(type(name) == "string" and type(fn) == "function", "Notice.listen(name, fn)")
  N.listeners[name], N.listen_errors[name] = fn, nil
end

function N.unlisten(name)
  N.listeners[name], N.listen_errors[name] = nil, nil
end

local frame = CreateFrame("Frame")
local since = 0
frame:SetScript("OnUpdate", function(_, dt)
  since = since + dt
  if since < N.every then return end
  since = 0
  pass()
end)

return N
