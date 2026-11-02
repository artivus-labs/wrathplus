-- wp.agent: where the agent's own Lua meets the fast decision tier. Loaded as the global `Agent`.
--
-- Intents: things the fast tier (Jev) may choose to do, registered by any module at runtime.
--   Agent.intent("pull_wolf", {
--     when = "Out of combat, healthy, and a living wolf is within 30 yards.",
--     available = function() return not UnitAffectingCombat("player") end,  -- optional filter
--     run = function() local w = V.nearest({ name = "Diseased Young Wolf", alive = true }) ... end,
--     every = 2,                                                             -- optional repeat gap
--   })
-- The fast tier lists the available intents every tick, lets Jev choose among them, and calls
-- the chosen one's run. Registering again replaces; Agent.remove(name) drops one.
--
-- Decisions: a tricky choice inside realtime code, handed up to the fast tier.
--   Agent.decide("flee_or_fight", {
--     question = "Two wolves are on us at half health. Fight on or run?",
--     options = { fight = "Keep attacking", flee = "Run back toward the guards" },
--     context = { health = V.health("player"), attackers = 2 },
--     on = function(choice) ... end,
--     default = "fight", timeout = 2,                -- used if no answer comes in time
--   })
-- The fast tier answers on its next tick; if it doesn't within `timeout` seconds, `default` is used.

local A = { intents = {}, pending = {}, next_id = 0, log = {} }

local function note(text)
  A.log[#A.log + 1] = string.format("%.1f %s", GetTime(), text)
  if #A.log > 50 then table.remove(A.log, 1) end
end

local function valid_name(name)
  return type(name) == "string" and name:match("^[a-z][a-z0-9_]*$") ~= nil
end

function A.intent(name, spec)
  assert(valid_name(name), "intent names are snake_case words")
  assert(type(spec) == "table" and type(spec.run) == "function", "intent " .. name .. " needs run")
  assert(type(spec.when) == "string", "intent " .. name .. " needs when (a description for the fast tier)")
  A.intents[name] = spec
  return spec
end

function A.remove(name) A.intents[name] = nil end

-- The intents whose `available` passes now, as { name, when } pairs, for the fast tier.
function A.available()
  local out = {}
  for name, spec in pairs(A.intents) do
    local ok, yes = true, true
    if spec.available then ok, yes = pcall(spec.available) end
    if ok and yes then out[#out + 1] = { name = name, when = spec.when, every = spec.every } end
  end
  table.sort(out, function(a, b) return a.name < b.name end)
  return out
end

-- Run an intent (called by the fast tier). Returns ok and its result or error.
function A.run(name)
  local spec = A.intents[name]
  if not spec then return false, "no intent " .. tostring(name) end
  local ok, result = pcall(spec.run)
  note((ok and "ran " or "failed ") .. name .. (ok and "" or ": " .. tostring(result)))
  return ok, result
end

local timer
local function expire()
  local now = GetTime()
  for id, d in pairs(A.pending) do
    if now >= d.deadline then
      A.pending[id] = nil
      note("decision " .. d.name .. " timed out: " .. tostring(d.default))
      if d.default ~= nil and d.on then pcall(d.on, d.default, "default") end
    end
  end
end

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
  if not timer then
    timer = CreateFrame("Frame")
    timer:SetScript("OnUpdate", expire)
  end
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
