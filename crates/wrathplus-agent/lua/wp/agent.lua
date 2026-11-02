-- wp.agent: where the agent's own Lua meets the fast decision tier. Loaded as the global `Agent`.
--
-- Intents are things the fast tier (Jev) may choose to do, registered by any module at runtime:
--   Agent.intent("clear_camp", {
--     when = "Out of combat and the quest still needs kobolds.",   -- what Jev reads
--     available = function() return not UnitAffectingCombat("player") end,  -- optional filter
--     run = function() Hunt:start() end,                         -- starts the activity
--     done = function() return Hunt.finished, Hunt.ok end,       -- optional: when it has ended
--     timeout = 120,                                             -- optional: give up after (s)
--   })
-- Choosing an intent starts an *activity*. Without `done` it ends as soon as `run` returns. With
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
  assert(type(spec.when) == "string", "intent " .. name .. " needs when (a description for the fast tier)")
  A.intents[name] = spec
  return spec
end

function A.remove(name)
  A.intents[name] = nil
  if A.current and A.current.name == name then A.current = nil end
end

-- The intents whose `available` passes now, as { name, when } pairs, for the fast tier.
function A.available()
  local out = {}
  for name, spec in pairs(A.intents) do
    local ok, yes = true, true
    if spec.available then ok, yes = pcall(spec.available) end
    if ok and yes then out[#out + 1] = { name = name, when = spec.when } end
  end
  table.sort(out, function(a, b) return a.name < b.name end)
  return out
end

local function finish(ok, why)
  local c = A.current
  if not c then return end
  A.current = nil
  note((ok and "done " or "ended ") .. c.name .. (why and (": " .. why) or ""))
  A.signal("INTENT_DONE", c.name, ok and true or false, why)
end

-- Start an intent's activity (called by the fast tier). Returns ok and run's result or error.
function A.run(name)
  local spec = A.intents[name]
  if not spec then return false, "no intent " .. tostring(name) end
  if A.current then finish(false, "replaced by " .. name) end
  A.current = { name = name, since = GetTime(), spec = spec }
  local ok, result = pcall(spec.run)
  if not ok then
    note("failed " .. name .. ": " .. tostring(result))
    finish(false, tostring(result))
    return false, result
  end
  if not spec.done then finish(true) end
  return true, result
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

-- One watcher frame: activity ends and decision timeouts, a few times a second.
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
    local c = A.current
    if c and c.spec.done then
      local ok, finished, succeeded = pcall(c.spec.done)
      if not ok then
        finish(false, "done check failed: " .. tostring(finished))
      elseif finished then
        finish(succeeded ~= false)
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

return A
