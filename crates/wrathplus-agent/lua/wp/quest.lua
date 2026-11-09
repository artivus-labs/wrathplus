-- wp.quest: the quest giver's dialogs, and keeping in step with the party's quests. Loaded for
-- every agent as the global `Quests`.
--
--   Quests.pick_up()   take every quest the quest givers within `reach` yards offer (the "!"
--                      over their heads): walk to each, through gossip or the quest greeting,
--                      accept, and ask again until none is left.
--   Quests.hand_in()   hand in every finished quest to the givers within `reach` (the yellow
--                      "?"): complete it and take the reward that is an upgrade (Upkeep), else
--                      the one that sells for most.
--                      Both return at once and run in the background; the visit ends as
--                      AGENT_QUESTS_DONE (kind, picked up, handed in), so wait on that, not the log.
--   Quests.state       what it is doing; Quests.busy() for an intent's `until`.
--   Quests.list()      the quest log: { { id, title, level, complete, objectives = { { text,
--                      kind, done }, ... } }, ... } (kind: "monster", "item", "object", "event"
--                      ...), headers skipped (GetQuestLogTitle's 3.3.5 order is
--                      title, level, tag, suggestedGroup, isHeader, ..., questID).
--   Quests.where(id)   where a logged quest's objectives are and where it is handed in, from the
--                      quest's map areas: { objectives = { { index, x, y }, ... }, hand_in = { x, y } }
--                      (nil until the client has them). Move.to(x, y) gets there.
--
-- The fast tier is offered both as intents (pick_up_quests, hand_in_quests) whenever such a quest
-- giver is near. Keeping in step: when the companion (Move.companion_now) picks up a quest or
-- hands one in, AGENT_PARTY_QUEST (name, "accepted" | "completed", quest id) wakes the fast tier;
-- a quest a party member shares is accepted (`accept_shared`). In a party, a visit ends with a
-- line in party chat of what it handed in and picked up (`announce`).

local Q = { reach = 30, accept_shared = true, announce = true, state = nil, log = {} }

local job, npc, since, asked
-- This visit's quests, for the line in party chat when it ends.
local visit = { accepted = {}, handed = {} }

function Q.where(id)
  local pois = AgentQuestPOI and AgentQuestPOI(id)
  if not pois then return nil end
  local out = { objectives = {} }
  for _, p in ipairs(pois) do
    if p.objective == -1 then
      out.hand_in = { x = p.x, y = p.y }
    else
      out.objectives[#out.objectives + 1] = { index = p.objective, x = p.x, y = p.y }
    end
  end
  return out
end

local function did(text)
  Q.log[#Q.log + 1] = text
  if #Q.log > 5 then table.remove(Q.log, 1) end
end

-- The nearest quest giver within reach whose marker says it has something of `kind` for us.
local function giver(kind)
  local best
  for _, u in ipairs(AgentNearby(Q.reach)) do
    if u.quest == kind and not u.dead and (not best or u.distance < best.distance) then best = u end
  end
  return best
end

-- In a party, say what the visit did, so the others need not ask: "Handed in: X. Picked up: Y."
local function announce()
  local parts = {}
  if #visit.handed > 0 then parts[#parts + 1] = "Handed in: " .. table.concat(visit.handed, ", ") .. "." end
  if #visit.accepted > 0 then parts[#parts + 1] = "Picked up: " .. table.concat(visit.accepted, ", ") .. "." end
  if Q.announce and #parts > 0 and (GetNumPartyMembers() or 0) > 0 then
    SendChatMessage(table.concat(parts, " "), "PARTY")
  end
  visit = { accepted = {}, handed = {} }
end

local function stop(why)
  if job then
    did(why)
    if Agent and Agent.signal then
      Agent.signal("QUESTS_DONE", job == "available" and "pick_up" or "hand_in", #visit.accepted, #visit.handed)
    end
    announce()
  end
  job, npc, Q.state, asked = nil, nil, nil, nil
  if GossipFrame and GossipFrame:IsShown() and CloseGossip then CloseGossip() end
end

local function start(kind)
  job, npc, since, asked = kind, nil, GetTime(), nil
  return true
end

function Q.pick_up() return start("available") end
function Q.hand_in() return start("reward") end

function Q.list()
  local out = {}
  for i = 1, GetNumQuestLogEntries() or 0 do
    local title, level, _, _, header, _, complete, _, id = GetQuestLogTitle(i)
    if title and not header then
      local objectives = {}
      for j = 1, GetNumQuestLeaderBoards(i) or 0 do
        local text, kind, done = GetQuestLogLeaderBoard(j, i)
        objectives[#objectives + 1] = { text = text, kind = kind, done = done and true or false }
      end
      out[#out + 1] = { id = id, title = title, level = level, complete = complete == 1, objectives = objectives }
    end
  end
  return out
end

function Q.busy()
  return job ~= nil
end

-- The reward to take: an upgrade we would wear, else the one that sells for most.
local function reward()
  local n = GetNumQuestChoices() or 0
  if n <= 1 then return 1 end
  local best, price = 1, -1
  for i = 1, n do
    local link = GetQuestItemLink("choice", i)
    if link and Upkeep and Upkeep.upgrade and Upkeep.upgrade(link) then return i end
    local value = link and select(11, GetItemInfo(link)) or 0
    if value > price then best, price = i, value end
  end
  return best
end

local frame = CreateFrame("Frame")
for _, event in ipairs({ "GOSSIP_SHOW", "QUEST_GREETING", "QUEST_DETAIL", "QUEST_PROGRESS",
  "QUEST_COMPLETE", "QUEST_ACCEPT_CONFIRM" }) do
  frame:RegisterEvent(event)
end
frame:SetScript("OnEvent", function(_, event)
  if event == "QUEST_ACCEPT_CONFIRM" then
    if Q.accept_shared then ConfirmAcceptQuest() end
    return
  end
  -- A quest a party member shares opens its details with them as the giver.
  if event == "QUEST_DETAIL" and not job and Q.accept_shared and UnitIsPlayer("questnpc")
    and (UnitInParty("questnpc") or UnitInRaid and UnitInRaid("questnpc")) then
    AcceptQuest()
    did("accepted a quest shared by " .. tostring(UnitName("questnpc")))
    return
  end
  if not job then return end
  if event == "GOSSIP_SHOW" then
    if job == "available" and (GetNumGossipAvailableQuests() or 0) > 0 then
      SelectGossipAvailableQuest(1)
    elseif job == "reward" then
      -- Four values per active quest: title, level, low level, complete.
      local active = { GetGossipActiveQuests() }
      for i = 1, #active / 4 do
        if active[i * 4] then return SelectGossipActiveQuest(i) end
      end
      CloseGossip()
    else
      CloseGossip()
    end
  elseif event == "QUEST_GREETING" then
    if job == "available" and (GetNumAvailableQuests() or 0) > 0 then
      SelectAvailableQuest(1)
    elseif job == "reward" then
      for i = 1, (GetNumActiveQuests() or 0) do
        local _, complete = GetActiveTitle(i)
        if complete then return SelectActiveQuest(i) end
      end
    end
  elseif event == "QUEST_DETAIL" and job == "available" then
    local title = GetTitleText()
    AcceptQuest()
    did("accepted " .. tostring(title))
    visit.accepted[#visit.accepted + 1] = tostring(title)
    asked = nil -- ask the giver again for the next one
  elseif event == "QUEST_PROGRESS" then
    if IsQuestCompletable() then CompleteQuest() end
  elseif event == "QUEST_COMPLETE" then
    local title = GetTitleText()
    GetQuestReward(reward())
    did("handed in " .. tostring(title))
    visit.handed[#visit.handed + 1] = tostring(title)
    asked = nil
  end
end)

-- The party's quests, to notice the companion's new and finished ones.
local seen_quests = {}
local function watch_party(now)
  local name = Move and Move.companion_now and Move.companion_now()
  if not name then return end
  for _, u in ipairs(AgentNearby(60)) do
    if u.kind == "player" and u.name == name and u.quests then
      local now_quests = {}
      for _, q in ipairs(u.quests) do now_quests[q.id] = true end
      local before = seen_quests[name]
      if before then
        for id in pairs(now_quests) do
          if not before[id] and Agent then Agent.signal("PARTY_QUEST", name, "accepted", id) end
        end
        for id in pairs(before) do
          if not now_quests[id] and Agent then Agent.signal("PARTY_QUEST", name, "completed", id) end
        end
      end
      seen_quests[name] = now_quests
    end
  end
end

local since_tick, since_party = 0, 0
frame:SetScript("OnUpdate", function(_, dt)
  since_tick, since_party = since_tick + dt, since_party + dt
  if since_party >= 1 then
    since_party = 0
    watch_party(GetTime())
  end
  if not job or since_tick < 0.5 then return end
  since_tick = 0
  local now = GetTime()
  if UnitAffectingCombat("player") then
    Q.state = "waiting for the fight to end"
    return
  end
  -- The giver we are on, or the next one with something for us.
  local target = npc and AgentNearby(Q.reach)
  local current
  if target then
    for _, u in ipairs(target) do
      if u.guid == npc and u.quest == job then current = u end
    end
  end
  current = current or giver(job)
  if not current then
    -- One visit does both, as a player would: hand in, then take what is on offer (or the other
    -- way round) before leaving.
    local other = job == "available" and "reward" or "available"
    if giver(other) then
      did(job == "available" and "picked up everything on offer" or "handed in everything finished")
      job, npc, asked = other, nil, nil
      return
    end
    return stop(job == "available" and "picked up everything on offer" or "handed in everything finished")
  end
  if now - since > 90 then return stop("gave up on " .. tostring(current.name) .. " after 90 s") end
  npc = current.guid
  -- Talk to them; again when a dialog closed without the marker changing.
  if not asked or now - asked >= 4 then
    asked = now
    AgentInteract(current.guid)
  end
  Q.state = (job == "available" and "picking up quests from " or "handing in quests to ") .. (current.name or "?")
end)

if Agent and Agent.intent then
  local function names(kind)
    local out = {}
    for _, u in ipairs(AgentNearby(Q.reach)) do
      if u.quest == kind and not u.dead then out[#out + 1] = u.name or "?" end
    end
    return table.concat(out, ", ")
  end
  Agent.intent("pick_up_quests", {
    when = function() return "Quest givers nearby have quests for you (" .. names("available") .. "): pick them up." end,
    available = function() return giver("available") ~= nil end,
    run = function() Q.pick_up() end,
    done = function() return not Q.busy() end,
    timeout = 120,
  })
  Agent.intent("hand_in_quests", {
    when = function() return "You have finished quests to hand in nearby (" .. names("reward") .. ")." end,
    available = function() return giver("reward") ~= nil end,
    run = function() Q.hand_in() end,
    done = function() return not Q.busy() end,
    timeout = 120,
  })
end

return Q
