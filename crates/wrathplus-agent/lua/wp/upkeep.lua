-- wp.upkeep: the chores a player does without thinking, so the tiers above need not. Loaded for
-- every agent as the global `Upkeep`; set `on`, `equip`, `sell` or `train` false to do one yourself.
--
--   Equip:  out of combat, wear what is in the bags when the slot is empty, or when it is a clear
--           upgrade (a higher item level, no lower quality) of the same armour type. Weapons and
--           shields only fill an empty slot (two-hander or sword and board is the agent's call),
--           and a shield never goes on beside a two-hander.
--   Sell:   at a merchant, sell grey (junk) items, and repair if the money covers it.
--   Train:  at a class trainer, learn every available rank the money covers, then close the
--           window. Upkeep.trainer() is the nearest trainer of your own class ("Warrior
--           Trainer"); after a level-up the fast tier is offered train_skills when one is in sight.
--   Loot:   in a group roll, `loot` = "upgrades" (the default) needs on what it would wear as an
--           upgrade and greeds on the rest; "greed" always greeds; "pass" passes; false leaves the
--           roll to the agent (START_LOOT_ROLL, RollOnLoot). A bind-on-pickup roll it made is
--           confirmed.
--
-- `Upkeep.log` keeps the last few things it did ("equipped Outfitter Belt", ...).

local U = { on = true, equip = true, sell = true, train = true, loot = "upgrades", log = {} }

local function did(text)
  U.log[#U.log + 1] = text
  if #U.log > 5 then table.remove(U.log, 1) end
end

-- Inventory slots per equip location (rings and trinkets take the first free of two).
local SLOTS = {
  INVTYPE_HEAD = { 1 }, INVTYPE_NECK = { 2 }, INVTYPE_SHOULDER = { 3 }, INVTYPE_BODY = { 4 },
  INVTYPE_CHEST = { 5 }, INVTYPE_ROBE = { 5 }, INVTYPE_WAIST = { 6 }, INVTYPE_LEGS = { 7 },
  INVTYPE_FEET = { 8 }, INVTYPE_WRIST = { 9 }, INVTYPE_HAND = { 10 }, INVTYPE_FINGER = { 11, 12 },
  INVTYPE_TRINKET = { 13, 14 }, INVTYPE_CLOAK = { 15 }, INVTYPE_WEAPON = { 16 },
  INVTYPE_2HWEAPON = { 16 }, INVTYPE_WEAPONMAINHAND = { 16 }, INVTYPE_SHIELD = { 17 },
  INVTYPE_WEAPONOFFHAND = { 17 }, INVTYPE_HOLDABLE = { 17 }, INVTYPE_RANGED = { 18 },
  INVTYPE_RANGEDRIGHT = { 18 }, INVTYPE_THROWN = { 18 }, INVTYPE_TABARD = { 19 },
}
local WEAPON_SLOTS = { [16] = true, [17] = true, [18] = true }

-- The heaviest armour a class wears, and the level it starts (3.3.5: mail and plate at 40).
local ARMOUR = { Cloth = 1, Leather = 2, Mail = 3, Plate = 4 }
local WEARS = {
  WARRIOR = { 3, 4 }, PALADIN = { 3, 4 }, DEATHKNIGHT = { 4, 4 }, HUNTER = { 2, 3 },
  SHAMAN = { 2, 3 }, ROGUE = { 2, 2 }, DRUID = { 2, 2 }, PRIEST = { 1, 1 }, MAGE = { 1, 1 },
  WARLOCK = { 1, 1 },
}

local function heaviest()
  local _, class = UnitClass("player")
  local w = WEARS[class or ""] or { 1, 1 }
  return (UnitLevel("player") or 1) >= 40 and w[2] or w[1]
end

-- An item's facts from its link: name, quality, item level, required level, class, subclass, slot.
local function info(link)
  if not link then return nil end
  local name, _, quality, ilevel, minlevel, class, subclass, _, equip, _, price = GetItemInfo(link)
  if not name then return nil end
  return { name = name, link = link, quality = quality or 0, ilevel = ilevel or 0, minlevel = minlevel or 0,
    class = class, subclass = subclass, equip = equip, price = price or 0 }
end

local function wearable(item)
  if not item.equip or not SLOTS[item.equip] then return false end
  if item.minlevel > (UnitLevel("player") or 1) then return false end
  if item.class == "Armor" and ARMOUR[item.subclass] and ARMOUR[item.subclass] > heaviest() then return false end
  return IsEquippableItem == nil or IsEquippableItem(item.link)
end

local function better(item, worn)
  if not worn then return true end
  if item.subclass ~= worn.subclass then return false end
  return item.ilevel > worn.ilevel and item.quality >= worn.quality
end

-- The best piece in the bags for each slot it could fill, and one of them worn per pass
-- (equipping changes the bags).
local function equip_pass()
  local main = info(GetInventoryItemLink("player", 16))
  local best = {}
  for bag = 0, 4 do
    for slot = 1, (GetContainerNumSlots(bag) or 0) do
      local item = info(GetContainerItemLink(bag, slot))
      if item and wearable(item) then
        for _, inv in ipairs(SLOTS[item.equip]) do
          local worn = info(GetInventoryItemLink("player", inv))
          local blocked = WEAPON_SLOTS[inv] and worn
            or (inv == 17 and main and main.equip == "INVTYPE_2HWEAPON")
          local rival = best[inv]
          if not blocked and better(item, worn)
            and (not rival or item.ilevel > rival.item.ilevel
              or (item.ilevel == rival.item.ilevel and item.quality > rival.item.quality)) then
            best[inv] = { item = item, bag = bag, slot = slot }
          end
        end
      end
    end
  end
  for inv = 1, 19 do
    local pick = best[inv]
    if pick then
      PickupContainerItem(pick.bag, pick.slot)
      EquipCursorItem(inv)
      did("equipped " .. pick.item.name)
      return true
    end
  end
  return false
end

local function sell_and_repair()
  local sold, earned = 0, 0
  for bag = 0, 4 do
    for slot = 1, (GetContainerNumSlots(bag) or 0) do
      local item = info(GetContainerItemLink(bag, slot))
      if item and item.quality == 0 and item.price > 0 then
        local _, count = GetContainerItemInfo(bag, slot)
        UseContainerItem(bag, slot)
        sold, earned = sold + 1, earned + item.price * (count or 1)
      end
    end
  end
  if sold > 0 then did(string.format("sold %d junk for %dc", sold, earned)) end
  if CanMerchantRepair and CanMerchantRepair() then
    local cost = GetRepairAllCost()
    if cost and cost > 0 and cost <= (GetMoney() or 0) + earned then
      RepairAllItems()
      did(string.format("repaired for %dc", cost))
    end
  end
end

-- The nearest trainer of our own class: a class trainer whose title names it ("Warrior Trainer").
function U.trainer(yards)
  local class = UnitClass("player")
  if not class then return nil end
  local best
  for _, u in ipairs(AgentNearby(yards or 150)) do
    local trains = false
    for _, s in ipairs(u.npc or {}) do if s == "classtrainer" then trains = true end end
    if trains and u.title and u.title:lower():find(class:lower(), 1, true) and (not best or u.distance < best.distance) then
      best = u
    end
  end
  return best
end

-- The level we last trained at; nil until a visit this session.
U.trained_level = nil

local function train()
  if IsTradeskillTrainer and IsTradeskillTrainer() then return end
  -- Not ours (a priest trainer for a warrior): nothing to learn here.
  if (GetNumTrainerServices() or 0) == 0 then return end
  local learned, money = {}, GetMoney() or 0
  local short, next_level = 0, nil -- what is too dear now, and when something new comes
  for i = 1, (GetNumTrainerServices() or 0) do
    local name, rank, category = GetTrainerServiceInfo(i)
    local cost = GetTrainerServiceCost(i) or 0
    if category == "available" and cost <= money then
      BuyTrainerService(i)
      money = money - cost
      learned[#learned + 1] = name .. (rank and rank ~= "" and (" (" .. rank .. ")") or "")
    elseif category == "available" then
      short = short + cost
    elseif category == "unavailable" and GetTrainerServiceLevelReq then
      local level = GetTrainerServiceLevelReq(i) or 0
      if level > (UnitLevel("player") or 0) and (not next_level or level < next_level) then next_level = level end
    end
  end
  local who = UnitName("npc") or "the trainer"
  if #learned > 0 then
    did("trained " .. table.concat(learned, ", "))
  elseif short > 0 then
    did(string.format("nothing affordable at %s: %dc of skills, %dc in hand", who, short, money))
  else
    did("nothing new at " .. who .. (next_level and (" until level " .. next_level) or ""))
  end
  U.trained_level = UnitLevel("player")
  -- Done here: close the window, or follow mode waits on it.
  if CloseTrainer then CloseTrainer() end
end

-- Would this item be worn as an upgrade in some slot?
local function upgrade(item)
  if not item or not wearable(item) then return false end
  for _, inv in ipairs(SLOTS[item.equip]) do
    if better(item, info(GetInventoryItemLink("player", inv))) then return true end
  end
  return false
end

-- For others (quest rewards): whether the item behind a link would be worn as an upgrade.
function U.upgrade(link)
  return upgrade(info(link))
end

-- 3.3.5 roll types: 0 pass, 1 need, 2 greed.
local ROLL_NAMES = { [0] = "passed on", [1] = "needed", [2] = "greeded" }
local rolled = {}

local function roll(id)
  local _, name, _, _, _, can_need, can_greed = GetLootRollItemInfo(id)
  if not name then return end
  local choice = 0
  if U.loot == "upgrades" and can_need and upgrade(info(GetLootRollItemLink(id))) then
    choice = 1
  elseif (U.loot == "upgrades" or U.loot == "greed") and can_greed then
    choice = 2
  end
  rolled[id] = choice
  RollOnLoot(id, choice)
  did(ROLL_NAMES[choice] .. " " .. name .. (choice == 1 and " (an upgrade)" or ""))
end

local frame = CreateFrame("Frame")
local since, dirty = 0, true
for _, event in ipairs({ "BAG_UPDATE", "PLAYER_LEVEL_UP", "MERCHANT_SHOW", "TRAINER_SHOW", "PLAYER_REGEN_ENABLED",
  "START_LOOT_ROLL", "CONFIRM_LOOT_ROLL" }) do
  frame:RegisterEvent(event)
end
frame:SetScript("OnEvent", function(_, event, a1, a2)
  if not U.on then return end
  if event == "START_LOOT_ROLL" and U.loot then
    roll(a1)
  elseif event == "CONFIRM_LOOT_ROLL" and rolled[a1] == a2 then
    -- Bind on pickup: confirm only a roll this module made.
    ConfirmLootRoll(a1, a2)
  elseif event == "MERCHANT_SHOW" and U.sell then
    sell_and_repair()
  elseif event == "TRAINER_SHOW" and U.train then
    train()
  else
    dirty = true
  end
end)
frame:SetScript("OnUpdate", function(_, dt)
  since = since + dt
  if since < 1 then return end
  since = 0
  if not (U.on and U.equip and dirty) then return end
  if UnitAffectingCombat("player") or UnitIsDeadOrGhost("player") then return end
  -- Equip until nothing more fits; the bags settle between pieces.
  dirty = equip_pass()
end)

-- After a level-up, with our trainer in sight: go and train (the fast tier chooses it).
if Agent and Agent.intent then
  local talking_since
  Agent.intent("train_skills", {
    when = function()
      local t = U.trainer(60)
      return "You have levelled since you last trained: learn your new skills from "
        .. (t and t.name or "your trainer") .. " (" .. (t and t.title or "") .. ")."
    end,
    -- Not in a fight: the walk to the trainer would only fail.
    available = function()
      return U.train and not UnitAffectingCombat("player") and (UnitLevel("player") or 1) > (U.trained_level or 1)
        and U.trainer(60) ~= nil
    end,
    run = function()
      talking_since = GetTime()
      local t = U.trainer(60)
      if t then AgentInteract(t.guid) end
    end,
    -- Over once trained at this level, or after a minute.
    done = function()
      if U.trained_level == UnitLevel("player") then return true, true end
      if talking_since and GetTime() - talking_since > 60 then return true, false, "could not reach the trainer" end
      return false
    end,
  })
end

return U
