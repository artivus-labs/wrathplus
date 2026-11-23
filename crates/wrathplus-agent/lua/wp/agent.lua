-- wp.agent: where the agent's own Lua meets the fast decision tier. Loaded as the global `Agent`.
--
-- Intents are things the fast tier (Jev) may choose to do, registered by any module at runtime:
--   Agent.intent("clear_camp", {
--     when = "Out of combat and the quest still needs kobolds.",   -- what Jev reads
--     available = function() return not UnitAffectingCombat("player") end,  -- optional filter
--     run = function() Hunt:start() end,                         -- starts the activity
--     done = function() return Hunt.finished, Hunt.ok end,       -- optional: when it has ended
--     timeout = 120,                                             -- optional: give up after (s)
--     while_dead = true,                                         -- optional: offered dead too
--   })
-- Choosing an intent starts an *activity*. Without `done` it ends, unannounced, as soon as `run`
-- returns (a failed `run` is still announced). With
-- `done`, it runs until `done()` returns true (or `timeout` passes), and the fast tier stays
-- asleep meanwhile: realtime Lua carries it. Its end is signalled (AGENT_INTENT_DONE), which wakes
-- the fast tier for the next choice. Registering again replaces; Agent.remove(name) drops one.
--
-- Decisions hand a tricky choice from realtime code up to the fast tier:
--   Agent.decide("flee_or_fight", {
--     question = "Two wolves are on us at half health. Fight on or run?",
--     options = { fight = "Keep attacking", flee = "Run back toward the guards" },
--     context = { health = V.health("player"), attackers = 2 },
--     on = function(choice) ... end,
--     default = "fight", timeout = 2,                -- used if no answer comes in time
--   })
-- Opening one signals AGENT_DECISION, which wakes the fast tier at once.
--
-- Asks hand a messy moment further up, to the LLM planner, with no fixed options (see A.ask):
--   Agent.ask(question, context, on[, { timeout, default }]) signals AGENT_ASK; the planner
--   answers with Agent.answer(id, value).
--
-- Agent.signal(name, ...) puts an event on the agent's event stream (AGENT_<NAME>), for anything
-- else the tiers above should hear about.

local A = { intents = {}, pending = {}, next_id = 0, log = {}, current = nil }

local function note(text)
  A.log[#A.log + 1] = string.format("%.1f %s", GetTime(), text)
  if #A.log > 50 then table.remove(A.log, 1) end
end

local function valid_name(name)
  return type(name) == "string" and name:match("^[a-z][a-z0-9_]*$") ~= nil
end

function A.signal(name, ...)
  if AgentSignal then AgentSignal(name, ...) end
end

function A.intent(name, spec)
  assert(valid_name(name), "intent names are snake_case words")
  assert(type(spec) == "table" and type(spec.run) == "function", "intent " .. name .. " needs run")
  assert(type(spec.when) == "string" or type(spec.when) == "function",
    "intent " .. name .. " needs when (a description for the fast tier)")
  A.intents[name] = spec
  return spec
end

function A.remove(name)
  A.intents[name] = nil
  if A.current and A.current.name == name then A.current = nil end
end

-- The intents whose `available` passes now, as { name, when } pairs, for the fast tier. Dead or a
-- ghost, nothing is offered (wp.revive carries that) but an intent marked `while_dead`.
function A.available()
  local out = {}
  local dead = UnitIsDeadOrGhost and UnitIsDeadOrGhost("player")
  for name, spec in pairs(A.intents) do
    local ok, yes = not dead or spec.while_dead == true, true
    if ok and spec.available then ok, yes = pcall(spec.available) end
    local when = spec.when
    if type(when) == "function" then
      local fine, text = pcall(when)
      when = fine and tostring(text) or name
    end
    if ok and yes then out[#out + 1] = { name = name, when = when } end
  end
  table.sort(out, function(a, b) return a.name < b.name end)
  return out
end

-- An activity's end is signalled; an instant intent (no `done`) that ran cleanly is not, or the
-- fast tier, woken by its "end", would choose it again at once in a loop.
local function finish(ok, why)
  local c = A.current
  if not c then return end
  A.current = nil
  note((ok and "done " or "ended ") .. c.name .. (why and (": " .. why) or ""))
  if c.spec.done or not ok then A.signal("INTENT_DONE", c.name, ok and true or false, why) end
end

-- Start an intent's activity (called by the fast tier). Returns ok and run's result or error.
-- Choosing the activity already running carries on with it: it never replaces itself.
function A.run(name)
  local spec = A.intents[name]
  if not spec then return false, "no intent " .. tostring(name) end
  if A.current and A.current.name == name then return true, "already doing " .. name end
  if A.current then finish(false, "replaced by " .. name) end
  A.current = { name = name, since = GetTime(), spec = spec }
  -- While an activity starts, A.starting names it: what it sets going is its own (wp.goals tells
  -- a walk an activity starts from one the planner starts by hand).
  local outer = A.starting
  A.starting = name
  local ok, result = pcall(spec.run)
  A.starting = outer
  if not ok then
    note("failed " .. name .. ": " .. tostring(result))
    finish(false, tostring(result))
    return false, result
  end
  if not spec.done then finish(true) end
  return true, result
end

-- End the running activity, as failed (a death, a new plan from code): signalled like any end.
function A.stop(why)
  finish(false, why)
end

-- The activity in progress: { name, seconds } or nil.
function A.activity()
  local c = A.current
  if not c then return nil end
  return { name = c.name, seconds = math.floor((GetTime() - c.since) * 10) / 10 }
end

local function decision_expiry(now)
  for id, d in pairs(A.pending) do
    if now >= d.deadline then
      A.pending[id] = nil
      note("decision " .. d.name .. " timed out: " .. tostring(d.default))
      if d.default ~= nil and d.on then pcall(d.on, d.default, "default") end
    end
  end
end

-- One watcher frame: activity ends, decision and ask timeouts, a few times a second.
local ask_expiry -- defined with the asks, below
local watcher
local since = 0
local function ensure_watcher()
  if watcher then return end
  watcher = CreateFrame("Frame")
  watcher:SetScript("OnUpdate", function(_, dt)
    since = since + dt
    if since < 0.2 then return end
    since = 0
    local now = GetTime()
    decision_expiry(now)
    ask_expiry(now)
    local c = A.current
    if c and c.spec.done then
      local ok, finished, succeeded, why = pcall(c.spec.done)
      if not ok then
        finish(false, "done check failed: " .. tostring(finished))
      elseif finished then
        finish(succeeded ~= false, why)
      elseif c.spec.timeout and now - c.since >= c.spec.timeout then
        finish(false, "timed out")
      end
    end
  end)
end
ensure_watcher()

function A.decide(name, spec)
  assert(valid_name(name), "decision names are snake_case words")
  assert(type(spec.options) == "table" and next(spec.options), "decision " .. name .. " needs options")
  -- One pending decision per name: asking again replaces the older question.
  for id, d in pairs(A.pending) do if d.name == name then A.pending[id] = nil end end
  A.next_id = A.next_id + 1
  local id = A.next_id
  A.pending[id] = {
    id = id, name = name, question = spec.question or name, options = spec.options,
    context = spec.context, on = spec.on, default = spec.default,
    deadline = GetTime() + (spec.timeout or 3),
  }
  A.signal("DECISION", name, id)
  return id
end

-- The open decisions, for the fast tier (without their callbacks).
function A.questions()
  local out = {}
  for id, d in pairs(A.pending) do
    out[#out + 1] = { id = id, name = d.name, question = d.question, options = d.options, context = d.context }
  end
  table.sort(out, function(a, b) return a.id < b.id end)
  return out
end

-- Answer a decision (called by the fast tier).
function A.resolve(id, choice)
  local d = A.pending[id]
  if not d then return false, "no open decision " .. tostring(id) end
  if d.options[choice] == nil then return false, "not an option: " .. tostring(choice) end
  A.pending[id] = nil
  note("decision " .. d.name .. ": " .. tostring(choice))
  if d.on then return pcall(d.on, choice, "decided") end
  return true
end

-- Asks hand a messy moment up to the LLM planner, with what led to it: someone talking, an odd
-- mechanic, a hard choice. Unlike a decision there are no fixed options; the answer is any value.
--   Agent.ask("A player asks us to follow them into the cave. Go?", { who = "Brann", level = 12 },
--     function(answer, how) ... end,          -- how: "answered" or "timeout"
--     { timeout = 120, default = "no" })      -- optional: `on(default, "timeout")` if no answer
-- Opening one signals AGENT_ASK(id, question, context, digest), digest being the recent events
-- and agent notes as lines; the planner answers with Agent.answer(id, value). Without a timeout
-- an ask stays open until answered. A timed-out ask signals AGENT_ASK_EXPIRED(id).
A.asks = {}
A.recent = {}

local RECENT_MAX, DIGEST_LINES = 30, 15

local function short(v)
  local s = tostring(v)
  return #s > 80 and (s:sub(1, 77) .. "...") or s
end

local function heard(event, ...)
  local parts = { string.format("%.1f %s", GetTime(), event) }
  for i = 1, math.min(select("#", ...), 4) do
    local v = select(i, ...)
    if v ~= nil and v ~= "" then parts[#parts + 1] = short(v) end
  end
  A.recent[#A.recent + 1] = table.concat(parts, " ")
  if #A.recent > RECENT_MAX then table.remove(A.recent, 1) end
end

-- What an ask's digest is drawn from: what a player would notice, not combat-log noise.
local HEARD = {
  "CHAT_MSG_SAY", "CHAT_MSG_YELL", "CHAT_MSG_WHISPER", "CHAT_MSG_PARTY", "CHAT_MSG_PARTY_LEADER",
  "CHAT_MSG_EMOTE", "CHAT_MSG_TEXT_EMOTE", "CHAT_MSG_MONSTER_SAY", "CHAT_MSG_MONSTER_YELL",
  "CHAT_MSG_MONSTER_WHISPER", "CHAT_MSG_SYSTEM", "UI_ERROR_MESSAGE", "PLAYER_DEAD",
  "PLAYER_REGEN_DISABLED", "PLAYER_REGEN_ENABLED", "PLAYER_LEVEL_UP", "PARTY_INVITE_REQUEST",
  "TRADE_SHOW", "DUEL_REQUESTED", "QUEST_ACCEPTED", "ZONE_CHANGED_NEW_AREA",
}
local listener = CreateFrame("Frame")
for _, event in ipairs(HEARD) do listener:RegisterEvent(event) end
listener:SetScript("OnEvent", function(_, event, ...) heard(event, ...) end)

-- The last lines of what happened lately (events heard and agent notes), oldest first.
function A.digest(lines)
  local all = {}
  for _, l in ipairs(A.recent) do all[#all + 1] = l end
  for _, l in ipairs(A.log) do all[#all + 1] = l end
  table.sort(all, function(a, b) return (tonumber(a:match("^%S+")) or 0) < (tonumber(b:match("^%S+")) or 0) end)
  local n = lines or DIGEST_LINES
  local out = {}
  for i = math.max(1, #all - n + 1), #all do out[#out + 1] = all[i] end
  return out
end

-- Ask the planner (an LLM, seconds to minutes away): returns the ask's id.
function A.ask(question, context, on, opts)
  assert(type(question) == "string" and question ~= "", "Agent.ask needs a question")
  assert(on == nil or type(on) == "function", "Agent.ask: on is a function")
  opts = opts or {}
  A.next_id = A.next_id + 1
  local id = A.next_id
  A.asks[id] = {
    id = id, question = question, context = context, on = on, default = opts.default,
    since = GetTime(), deadline = opts.timeout and (GetTime() + opts.timeout) or nil,
  }
  note("ask " .. id .. ": " .. short(question))
  A.signal("ASK", id, question, context, A.digest())
  return id
end

-- Answer an ask (called by the planner): `on(value, "answered")` runs and its results return.
function A.answer(id, value)
  local q = A.asks[id]
  if not q then return false, "no open ask " .. tostring(id) end
  A.asks[id] = nil
  note("answer " .. id .. ": " .. short(value))
  if q.on then return pcall(q.on, value, "answered") end
  return true
end

-- The open asks, oldest first (without their callbacks), for a planner that missed AGENT_ASK.
function A.open_asks()
  local out = {}
  for id, q in pairs(A.asks) do
    out[#out + 1] = { id = id, question = q.question, context = q.context,
      seconds = math.floor((GetTime() - q.since) * 10) / 10 }
  end
  table.sort(out, function(a, b) return a.id < b.id end)
  return out
end

ask_expiry = function(now)
  for id, q in pairs(A.asks) do
    if q.deadline and now >= q.deadline then
      A.asks[id] = nil
      note("ask " .. id .. " timed out: " .. short(q.default))
      A.signal("ASK_EXPIRED", id)
      if q.on then pcall(q.on, q.default, "timeout") end
    end
  end
end

-- Steps: an ordered list from the planner ("walk here, then there, then talk to X"). They run as
-- one activity, `follow_steps`, which moves to the next step by itself as each one's `done`
-- passes, so the tiers above sleep through the whole list. It ends when every step is done, or a
-- step fails or times out (the reason names the step); another activity (a fight) interrupts it,
-- and choosing it again resumes at the step it was on.
--   Agent.set_steps({ { name = "to the lake", run = function() AgentMoveTo(-9400, -90) end,
--                       done = function() return AgentWalk().status == "arrived" end, timeout = 120 }, ... })
A.steps = { list = {}, at = 0 }

local function step_now()
  return A.steps.list[A.steps.at]
end

local function start_step()
  local step = step_now()
  if not step then return end
  A.steps.since = GetTime()
  note("step " .. A.steps.at .. ": " .. step.name)
  step.run()
end

local function step_check()
  local step = step_now()
  if not step then return true, true end
  local finished, succeeded = true, true
  if step.done then
    local ok, f, s2 = pcall(step.done)
    if not ok then return true, false, "step " .. step.name .. ": done check failed: " .. tostring(f) end
    finished, succeeded = f, s2
  end
  if not finished then
    if step.timeout and GetTime() - (A.steps.since or GetTime()) >= step.timeout then
      A.steps.failed = A.steps.at
      return true, false, "step " .. step.name .. " timed out"
    end
    return false
  end
  -- A failed step is the planner's to fix: follow_steps is not offered again until it sets new
  -- steps, or the fast tier re-runs the same failing step over and over.
  if succeeded == false then
    A.steps.failed = A.steps.at
    return true, false, "step " .. step.name .. " failed"
  end
  A.steps.at = A.steps.at + 1
  if not step_now() then
    A.steps.list, A.steps.at = {}, 0
    A.intents.follow_steps = nil
    return true, true, "all steps done"
  end
  local ok, err = pcall(start_step)
  if not ok then return true, false, "step " .. step_now().name .. ": " .. tostring(err) end
  return false
end

function A.set_steps(list)
  A.steps.list, A.steps.at = list or {}, (list and #list > 0) and 1 or 0
  A.steps.failed = nil
  if A.steps.at == 0 then
    A.intents.follow_steps = nil
    return
  end
  A.intents.follow_steps = {
    when = function()
      local step = step_now()
      return step and string.format("Carry on with the planner's steps: step %d of %d, %s.",
        A.steps.at, #A.steps.list, step.name) or "No steps left."
    end,
    available = function() return not A.steps.failed end,
    run = start_step,
    done = step_check,
    plan = true,
  }
end

return A
