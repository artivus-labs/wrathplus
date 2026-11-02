-- wp.engine: a small decision engine for agent-written game logic, modelled on mod-playerbots'
-- (src/Bot/Engine): triggers queue actions at a relevance, each tick runs the most relevant action
-- that is useful and possible, and actions chain prerequisites, alternatives and continuers.
-- Everything is a plain table, so an agent can write, inspect and change it at runtime.
--
--   local E = require("wp.engine")
--   local bot = E.new({ interval = 0.2 })
--   bot:value("hp", function() return UnitHealth("player") / UnitHealthMax("player") end, 0.5)
--   bot:trigger("low health", { check = function(c) return c:value("hp") < 0.4 end })
--   bot:action("eat", { useful = function(c) return not UnitAffectingCombat("player") end,
--                       execute = function(c) UseItemByName("Tough Jerky") return true end,
--                       duration = 20 })
--   bot:strategy("survive", { triggers = { { "low health", { { "eat", E.HIGH } } } } })
--   bot:enable("+survive")
--   bot:start()
--
-- Relevance follows Playerbots: IDLE 0, DEFAULT 5, NORMAL 10, HIGH 20, MOVE 30, INTERRUPT 40,
-- DISPEL 50, RAID 60, EMERGENCY 90, PASSTHROUGH 100.

local E = {
  IDLE = 0, DEFAULT = 5, NORMAL = 10, HIGH = 20, MOVE = 30, INTERRUPT = 40,
  DISPEL = 50, RAID = 60, EMERGENCY = 90, PASSTHROUGH = 100,
}

local Engine = {}
Engine.__index = Engine

-- How long a queued action waits before it is dropped (Playerbots' ExpireActionTime).
local EXPIRE = 5
-- How many actions a tick may consider before giving up (Playerbots: queue size x 10).
local ITERATIONS = 10

local function now() return GetTime() end

-- A new engine. opts.interval: seconds between ticks (default 0.1); opts.state: a function
-- returning the current state name (default: "dead", "combat" or "noncombat").
function E.new(opts)
  opts = opts or {}
  local self = setmetatable({
    interval = opts.interval or 0.1,
    actions = {}, triggers = {}, values = {}, strategies = {},
    -- Enabled strategy names per state, in enable order.
    enabled = { combat = {}, noncombat = {}, dead = {}, any = {} },
    queue = {}, memory = {}, busy_until = 0, log = {}, log_size = opts.log_size or 50,
    state_of = opts.state,
  }, Engine)
  return self
end

-- Register an action. spec: execute(ctx, event) -> bool (required); useful(ctx), possible(ctx)
-- -> bool (default true); prerequisites, alternatives, continuers: lists of action names;
-- duration: seconds the engine stays busy after a successful run (a cast, a meal).
function Engine:action(name, spec)
  assert(type(spec.execute) == "function", "action " .. name .. " needs execute")
  self.actions[name] = spec
  return self
end

-- Register a trigger. spec: check(ctx) -> event (any non-nil value fires) ; interval: seconds
-- between checks (default every tick).
function Engine:trigger(name, spec)
  assert(type(spec.check) == "function", "trigger " .. name .. " needs check")
  spec.next_check = 0
  self.triggers[name] = spec
  return self
end

-- A trigger fired by a game event: true once after the event, with its arguments as the event.
function Engine:on_event(name, event, filter)
  local pending
  self.event_frame = self.event_frame or CreateFrame("Frame")
  self.event_handlers = self.event_handlers or {}
  self.event_handlers[event] = self.event_handlers[event] or {}
  table.insert(self.event_handlers[event], function(...)
    if not filter or filter(...) then pending = { event, ... } end
  end)
  self.event_frame:RegisterEvent(event)
  local handlers = self.event_handlers
  self.event_frame:SetScript("OnEvent", function(_, ev, ...)
    for _, h in ipairs(handlers[ev] or {}) do h(...) end
  end)
  return self:trigger(name, { check = function()
    local fired = pending
    pending = nil
    return fired
  end })
end

-- A cached value: fn(ctx) recomputed at most every ttl seconds (default every read).
function Engine:value(name, fn, ttl)
  self.values[name] = { fn = fn, ttl = ttl or 0, at = -math.huge }
  return self
end

-- A strategy: { triggers = { {trigger, { {action, relevance}, ... }}, ... },
--   defaults = { {action, relevance}, ... }, multipliers = { function(action, ctx) -> factor },
--   state = "combat" | "noncombat" | "dead" | "any" (default "any") }.
function Engine:strategy(name, spec)
  spec.state = spec.state or "any"
  self.strategies[name] = spec
  return self
end

-- "+tank,-dps,~follow": add, remove or toggle strategies; "!" clears all first.
function Engine:enable(change)
  for part in string.gmatch(change, "[^,]+") do
    local op, name = part:match("^%s*([%+%-~!]?)%s*(.-)%s*$")
    if op == "!" then
      for state in pairs(self.enabled) do self.enabled[state] = {} end
    end
    if name and name ~= "" then
      local spec = assert(self.strategies[name], "no strategy " .. name)
      local list = self.enabled[spec.state]
      local at
      for i, n in ipairs(list) do if n == name then at = i end end
      if op == "-" or (op == "~" and at) then
        if at then table.remove(list, at) end
      elseif not at then
        table.insert(list, name)
      end
    end
  end
  return self
end

-- The enabled strategy names for the current state (plus "any").
function Engine:active()
  local state = self:state()
  local out = {}
  for _, n in ipairs(self.enabled.any) do out[#out + 1] = n end
  for _, n in ipairs(self.enabled[state] or {}) do out[#out + 1] = n end
  return out, state
end

function Engine:state()
  if self.state_of then return self.state_of(self) end
  if UnitIsDeadOrGhost("player") then return "dead" end
  if UnitAffectingCombat("player") then return "combat" end
  return "noncombat"
end

-- The context handed to every check: values, memory and the engine.
local Context = {}
Context.__index = Context

function Context:value(name)
  local v = self.engine.values[name]
  if not v then return self.engine.memory[name] end
  local t = now()
  if t - v.at >= v.ttl then
    v.cached = v.fn(self)
    v.at = t
  end
  return v.cached
end

-- Remember something (a settable value: Playerbots' blackboard).
function Context:set(name, value) self.engine.memory[name] = value end
function Context:get(name) return self.engine.memory[name] end

function Engine:context()
  self.ctx = self.ctx or setmetatable({ engine = self }, Context)
  return self.ctx
end

-- Queue an action; a name already queued keeps the higher relevance (Playerbots' Queue::Push).
function Engine:push(name, relevance, event, skip_prerequisites)
  for _, q in ipairs(self.queue) do
    if q.name == name then
      if relevance > q.relevance then
        q.relevance, q.event, q.skip = relevance, event, skip_prerequisites
      end
      return
    end
  end
  table.insert(self.queue, { name = name, relevance = relevance, event = event,
    skip = skip_prerequisites, at = now() })
end

function Engine:pop()
  local best, at
  for i, q in ipairs(self.queue) do
    if not best or q.relevance > best.relevance then best, at = q, i end
  end
  if at then table.remove(self.queue, at) end
  return best
end

function Engine:note(text)
  table.insert(self.log, string.format("%.1f %s", now(), text))
  if #self.log > self.log_size then table.remove(self.log, 1) end
end

-- One decision: fire due triggers, queue defaults, then run the most relevant useful, possible
-- action. Returns the name of the action that ran, if any.
function Engine:tick()
  local t = now()
  local ctx = self:context()
  -- Expired entries go.
  for i = #self.queue, 1, -1 do
    if t - self.queue[i].at > EXPIRE then table.remove(self.queue, i) end
  end
  if t < self.busy_until then return nil end
  local active = self:active()
  for _, sname in ipairs(active) do
    local s = self.strategies[sname]
    for _, node in ipairs(s.triggers or {}) do
      local trig = self.triggers[node[1]]
      if trig and t >= trig.next_check then
        trig.next_check = t + (trig.interval or 0)
        local ok, event = pcall(trig.check, ctx)
        if not ok then
          self:note("trigger " .. node[1] .. " failed: " .. tostring(event))
        elseif event then
          for _, next_action in ipairs(node[2]) do
            self:push(next_action[1], next_action[2] or E.NORMAL, event)
          end
        end
      end
    end
    for _, d in ipairs(s.defaults or {}) do self:push(d[1], d[2] or E.DEFAULT) end
  end
  for _ = 1, math.max(1, #self.queue * ITERATIONS) do
    local q = self:pop()
    if not q then return nil end
    local a = self.actions[q.name]
    local relevance = q.relevance
    if a then
      for _, sname in ipairs(active) do
        for _, m in ipairs(self.strategies[sname].multipliers or {}) do
          relevance = relevance * m(q.name, ctx)
        end
      end
    end
    if a and relevance > 0 and (not a.useful or a.useful(ctx)) then
      if not a.possible or a.possible(ctx) then
        if not q.skip and a.prerequisites and #a.prerequisites > 0 then
          for _, p in ipairs(a.prerequisites) do self:push(p, relevance + 0.002, q.event) end
          self:push(q.name, relevance + 0.001, q.event, true)
        else
          local ok, done = pcall(a.execute, ctx, q.event)
          if ok and done then
            self:note("ran " .. q.name)
            self.last = q.name
            if a.duration then self.busy_until = now() + a.duration end
            for _, c in ipairs(a.continuers or {}) do self:push(c, relevance, q.event) end
            return q.name
          end
          self:note((ok and "failed " or "error in ") .. q.name .. (ok and "" or ": " .. tostring(done)))
          for _, alt in ipairs(a.alternatives or {}) do self:push(alt, relevance + 0.003, q.event) end
        end
      else
        for _, alt in ipairs(a.alternatives or {}) do self:push(alt, relevance + 0.003, q.event) end
      end
    end
  end
  return nil
end

-- Tick on a timer from a hidden frame's OnUpdate; stop() ends it.
function Engine:start()
  self.frame = self.frame or CreateFrame("Frame")
  local elapsed = 0
  self.frame:SetScript("OnUpdate", function(_, dt)
    elapsed = elapsed + dt
    if elapsed >= self.interval then
      elapsed = 0
      local ok, err = pcall(self.tick, self)
      if not ok then self:note("tick error: " .. tostring(err)) end
    end
  end)
  self.running = true
  return self
end

function Engine:stop()
  if self.frame then self.frame:SetScript("OnUpdate", nil) end
  self.running = false
  return self
end

-- A summary for an agent or a person: state, active strategies, queue and recent decisions.
function Engine:describe()
  local active, state = self:active()
  local queue = {}
  for _, q in ipairs(self.queue) do queue[#queue + 1] = q.name .. "@" .. q.relevance end
  return { state = state, active = active, queue = queue, last = self.last,
    running = self.running, log = self.log }
end

return E
