-- The build-12340 engine API that the mature 1.12 surface lacks, over the state that surface
-- keeps. Systems this client does not have (Battle.net, voice chat, the dungeon finder, arena
-- teams, achievements, the calendar, vehicles, companions) answer as a client with nothing in
-- them would: no entries, not available. `__azb_fire_event` is the host's event dispatch.

__azb_fire_event = nil

local function none() end
local function zero() return 0 end

-- ── CVars ──────────────────────────────────────────────────────────────────────────────────────

-- ── talents, titles, flags ─────────────────────────────────────────────────────────────────────
IsStereoVideoAvailable = IsStereoVideoAvailable or none

-- ── Battle.net, voice chat ─────────────────────────────────────────────────────────────────────
BNFeaturesEnabled, BNFeaturesEnabledAndConnected, BNConnected = none, none, none
BNGetNumFriends = function() return 0, 0 end
BNGetNumFriendInvites, BNGetNumBlocked, BNGetNumBlockedToons = zero, zero, zero
BNGetMaxPlayersInConversation = zero
BNGetNumConversationMembers = zero
BNGetFriendInfo, BNGetFriendInfoByID, BNGetToonInfo, BNGetFriendInviteInfo = none, none, none, none
BNGetConversationInfo, BNGetCustomMessageTable, BNGetInfo = none, none, none
IsVoiceChatEnabled, IsVoiceChatAllowed, IsVoiceChatAllowedByServer = none, none, none
VoiceIsDisabledByClient = function() return 1 end
GetNumVoiceSessions = zero
GetVoiceCurrentSessionID, GetVoiceStatus = none, none
-- GetVoiceSessionInfo(index) (`0x7db0b0`): no voice session, so nil, one value.
function GetVoiceSessionInfo() return nil end
Sound_ChatSystem_GetNumInputDrivers, Sound_ChatSystem_GetNumOutputDrivers = zero, zero
Sound_ChatSystem_GetInputDriverNameByIndex = none
Sound_ChatSystem_GetOutputDriverNameByIndex = none
Sound_GameSystem_GetNumOutputDrivers = Sound_GameSystem_GetNumOutputDrivers or zero
Sound_GameSystem_GetOutputDriverNameByIndex = none

-- ── misc build-12340 queries ───────────────────────────────────────────────────────────────────
GetActiveVoiceChannel, GetNumVoiceSessionMembersBySessionID = none, zero

-- ── the stock UI's CVars, at build 12340's defaults, for any the host has not registered ────────
-- Audited against 12340's CVar::Register calls (2026-10-25); `locale` keeps the install's own.
do
    local stock = {
    addFriendInfoShown = "0",
    advancedWorldMap = "0",
    alwaysCompareItems = "0",
    alwaysShowActionBars = "0",
    assistAttack = "0",
    auctionDisplayOnCharacter = "0",
    autoClearAFK = "1",
    autoDismountFlying = "0",
    autointeract = "0",
    autojoinBGVoice = "0",
    autojoinPartyVoice = "1",
    autoLootDefault = "0",
    autoQuestProgress = "1",
    autoQuestWatch = "1",
    autoRangedCombat = "1",
    autoSelfCast = "1",
    BaseMip = "0",
    blockTrades = "0",
    buffDurations = "1",
    calendarShowBattlegrounds = "0",
    calendarShowDarkmoon = "1",
    calendarShowLockouts = "1",
    calendarShowResets = "0",
    calendarShowWeeklyHolidays = "1",
    cameraBobbing = "0",
    cameraDistanceMaxFactor = "1",
    cameraPitchMoveSpeed = "90",
    cameraPitchSmoothSpeed = "45",
    cameraPivot = "0",
    cameraSmoothStyle = "4",
    cameraSmoothTrackingStyle = "4",
    cameraTerrainTilt = "0",
    cameraWaterCollision = "0",
    cameraYawMoveSpeed = "180",
    cameraYawSmoothSpeed = "180",
    ChatAmbienceVolume = "0.3",
    chatBubbles = "1",
    chatBubblesParty = "1",
    chatMouseScroll = "1",
    ChatMusicVolume = "0.3",
    ChatSoundVolume = "0.4",
    chatStyle = "im",
    colorblindMode = "0",
    CombatDamage = "1",
    CombatHealing = "1",
    CombatLogPeriodicSpells = "1",
    combatTextFloatMode = "1",
    componentTextureLevel = "8",
    consolidateBuffs = "0",
    conversationMode = "popout",
    deselectOnClick = "0",
    desktopGamma = "1",
    displayFreeBagSlots = "0",
    displayWorldPVPObjectives = "1",
    enableCombatText = "1",
    EnableMicrophone = "1",
    enablePVPNotifyAFK = "1",
    EnableVoiceChat = "0",
    enableWoWMouse = "0",
    environmentDetail = "1",
    equipmentManager = "0",
    extShadowQuality = "0",
    farclip = "1000",
    fctAuras = "0",
    fctCombatState = "0",
    fctComboPoints = "0",
    fctDamageReduction = "0",
    fctDodgeParryMiss = "0",
    fctEnergyGains = "0",
    fctFriendlyHealers = "0",
    fctHonorGains = "0",
    fctLowManaHealth = "1",
    fctPeriodicEnergyGains = "0",
    fctReactives = "0",
    fctRepChanges = "0",
    fctSpellMechanics = "0",
    fctSpellMechanicsOther = "0",
    ffxDeath = "0",
    ffxGlow = "0",
    fullSizeFocusFrame = "0",
    groundEffectDensity = "16",
    groundEffectDist = "70",
    guildMemberNotify = "1",
    guildRecruitmentChannel = "1",
    gxCursor = "0",
    gxFixLag = "0",
    gxMaximize = "0",
    gxRefresh = "0",
    gxStereoConvergence = "1",
    gxStereoEnabled = "0",
    gxStereoSeparation = "25",
    gxTripleBuffer = "0",
    gxVSync = "0",
    gxWindow = "1",
    hidePartyInRaid = "0",
    InboundChatVolume = "1",
    iTunesRemoteFeedback = "0",
    iTunesTrackDisplay = "0",
    lastTalkedToGM = "",
    lfgDebug = "0",
    locale = "enUS",
    lockActionBars = "1",
    lootUnderMouse = "1",
    mapQuestDifficulty = "0",
    minimapTrackedInfo = "",
    miniWorldMap = "0",
    mouseInvertPitch = "0",
    mouseSpeed = "1",
    MovieRecordingAutoCompress = "0",
    MovieRecordingCompression = "0",
    MovieRecordingCursor = "0",
    MovieRecordingFramerate = "29.97",
    MovieRecordingGUI = "0",
    MovieRecordingIcon = "0",
    MovieRecordingQuality = "2",
    MovieRecordingRecover = "0",
    MovieRecordingSound = "0",
    MovieRecordingWidth = "1280",
    movieSubtitle = "0",
    nameplateAllowOverlap = "1",
    nameplateShowEnemies = "0",
    nameplateShowEnemyGuardians = "0",
    nameplateShowEnemyPets = "0",
    nameplateShowEnemyTotems = "0",
    nameplateShowFriendlyGuardians = "0",
    nameplateShowFriendlyPets = "0",
    nameplateShowFriendlyTotems = "0",
    nameplateShowFriends = "0",
    OutboundChatVolume = "1",
    particleDensity = "1",
    partyBackgroundOpacity = "0.5",
    partyStatusText = "0",
    pendingInviteInfoShown = "0",
    PetMeleeDamage = "1",
    PetSpellDamage = "1",
    petStatusText = "0",
    playerStatLeftDropdown = "PLAYERSTAT_BASE_STATS",
    playerStatRightDropdown = "PLAYERSTAT_MELEE_COMBAT",
    playerStatusText = "0",
    predictedHealth = "1",
    predictedPower = "1",
    previewTalents = "0",
    profanityFilter = "1",
    projectedTextures = "0",
    -- `0x4d11ed`: the grave key, `0x9e4104`.
    PushToTalkButton = "`",
    PushToTalkSound = "0",
    questFadingDisable = "1",
    questPOI = "1",
    removeChatDelay = "0",
    rotateMinimap = "0",
    screenEdgeFlash = "1",
    scriptErrors = "0",
    secureAbilityToggle = "1",
    showArenaEnemyCastbar = "1",
    showArenaEnemyFrames = "1",
    showArenaEnemyPets = "1",
    showBattlefieldMinimap = "1",
    showCastableBuffs = "0",
    showCastableDebuffs = "0",
    ShowClassColorInNameplate = "0",
    showClock = "1",
    showDispelDebuffs = "0",
    showGameTips = "1",
    showItemLevel = "0",
    showKeyring = "0",
    showLootSpam = "1",
    showNewbieTips = "1",
    showPartyBackground = "0",
    showPartyPets = "1",
    showRaidRange = "0",
    showTargetCastbar = "1",
    showTargetOfTarget = "0",
    showTimestamps = "none",
    showToastBroadcast = "1",
    showToastConversation = "1",
    showToastFriendRequest = "1",
    showToastOffline = "1",
    showToastOnline = "1",
    showToastWindow = "1",
    showTokenFrame = "0",
    showTokenFrameHonor = "0",
    showTutorials = "1",
    showVKeyCastbar = "1",
    Sound_AmbienceVolume = "0.6",
    Sound_EnableAllSound = "1",
    Sound_EnableAmbience = "1",
    Sound_EnableDSPEffects = "0",
    Sound_EnableEmoteSounds = "1",
    Sound_EnableErrorSpeech = "1",
    Sound_EnableHardware = "0",
    Sound_EnableMusic = "1",
    Sound_EnablePetSounds = "1",
    Sound_EnableReverb = "0",
    Sound_EnableSFX = "1",
    Sound_EnableSoftwareHRTF = "0",
    Sound_EnableSoundWhenGameIsInBG = "0",
    Sound_MasterVolume = "1",
    Sound_MusicVolume = "0.4",
    Sound_NumChannels = "32",
    Sound_OutputDriverIndex = "0",
    Sound_OutputQuality = "1",
    Sound_SFXVolume = "1",
    Sound_VoiceChatInputDriverIndex = "0",
    Sound_VoiceChatOutputDriverIndex = "0",
    Sound_ZoneMusicNoDelay = "0",
    spamFilter = "1",
    specular = "0",
    statusTextPercentage = "0",
    stopAutoAttackOnTargetChange = "0",
    talentFrameShown = "0",
    targetOfTargetMode = "5",
    targetStatusText = "0",
    textureFilteringMode = "1",
    threatPlaySounds = "1",
    threatShowNumeric = "0",
    threatWarning = "3",
    timeMgrAlarmEnabled = "0",
    timeMgrUseLocalTime = "0",
    timeMgrUseMilitaryTime = "0",
    toastDuration = "4",
    -- `0x51e0bc`-`0x51e0fd`: the tracker filter's three show bits (achievements, completed
    -- quests, remote zones) all set, and manual sorting. Filter "0" hid every watch off the
    -- current map's POI set, so the stock tracker counted them and drew no line.
    trackerFilter = "7",
    trackerSorting = "0",
    UberTooltips = "1",
    uiScale = "1",
    UnitNameEnemyGuardianName = "0",
    UnitNameEnemyPetName = "0",
    UnitNameEnemyPlayerName = "1",
    UnitNameEnemyTotemName = "0",
    UnitNameFriendlyGuardianName = "0",
    UnitNameFriendlyPetName = "0",
    UnitNameFriendlyPlayerName = "1",
    UnitNameFriendlyTotemName = "0",
    UnitNameNonCombatCreatureName = "0",
    UnitNameNPC = "0",
    UnitNameOwn = "0",
    UnitNamePlayerGuild = "1",
    UnitNamePlayerPVPTitle = "1",
    useEnglishAudio = "0",
    useUIScale = "0",
    VoiceActivationSensitivity = "0.4",
    VoiceChatMode = "0",
    VoiceChatSelfMute = "0",
    watchFrameWidth = "0",
    weatherDensity = "2",
    wholeChatWindowClickable = "1",
    windowResizeLock = "0",
    worldMapOpacity = "0",
    xpBarText = "0",
    }
    for name, default in pairs(stock) do
        RegisterCVar(name, default)
    end
end

-- ── class names, action and pet-bar predicates, group queries, chat window geometry ───────────

-- ── the next batch: static constants, multi-cast bar, bags, world map, achievements ────────────
IsPlayerResolutionAvailable = IsPlayerResolutionAvailable or function() return 1 end

-- ── vehicles, tutorials, guild log, friends ────────────────────────────────────────────────────
UnitIsTalking = UnitIsTalking or none

-- ── the engine's combat-log unit-flag constants (the store is native, wrath_combat_log.rs) ─────
COMBATLOG_OBJECT_AFFILIATION_MINE, COMBATLOG_OBJECT_AFFILIATION_PARTY = 0x1, 0x2
COMBATLOG_OBJECT_AFFILIATION_RAID, COMBATLOG_OBJECT_AFFILIATION_OUTSIDER = 0x4, 0x8
COMBATLOG_OBJECT_AFFILIATION_MASK = 0xF
COMBATLOG_OBJECT_REACTION_FRIENDLY, COMBATLOG_OBJECT_REACTION_NEUTRAL = 0x10, 0x20
COMBATLOG_OBJECT_REACTION_HOSTILE, COMBATLOG_OBJECT_REACTION_MASK = 0x40, 0xF0
COMBATLOG_OBJECT_CONTROL_PLAYER, COMBATLOG_OBJECT_CONTROL_NPC = 0x100, 0x200
COMBATLOG_OBJECT_CONTROL_MASK = 0x300
COMBATLOG_OBJECT_TYPE_PLAYER, COMBATLOG_OBJECT_TYPE_NPC = 0x400, 0x800
COMBATLOG_OBJECT_TYPE_PET, COMBATLOG_OBJECT_TYPE_GUARDIAN = 0x1000, 0x2000
COMBATLOG_OBJECT_TYPE_OBJECT, COMBATLOG_OBJECT_TYPE_MASK = 0x4000, 0xFC00
COMBATLOG_OBJECT_TARGET, COMBATLOG_OBJECT_FOCUS = 0x10000, 0x20000
COMBATLOG_OBJECT_MAINTANK, COMBATLOG_OBJECT_MAINASSIST = 0x40000, 0x80000
for i = 1, 8 do _G["COMBATLOG_OBJECT_RAIDTARGET" .. i] = 2 ^ (19 + i) end
COMBATLOG_OBJECT_RAIDTARGET_MASK = 0x0FF00000
COMBATLOG_OBJECT_NONE, COMBATLOG_OBJECT_SPECIAL_MASK = 0x80000000, 0xFFFF0000


