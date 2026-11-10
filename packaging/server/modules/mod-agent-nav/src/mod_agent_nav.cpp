// mod-agent-nav: walking routes for agent clients, answered by the realm.
//
// A WrathPlus agent client walks along navmesh routes. The navmesh (mmaps) only comes from a
// multi-hour extraction of the player's own game files, so most agents on a public realm have
// none; the realm has it loaded anyway. A logged-in character asks, in addon messages on its own
// game connection (AgentNavProtocol.h), for a route on the map it stands on, and gets the corners
// back.
//
// Threads. The request arrives in the chat handler (the world thread: CMSG_MESSAGECHAT is
// PROCESS_THREADUNSAFE). There it is only parsed, checked against the limits and queued. The
// search runs on this module's own thread, over this module's own dtNavMesh and dtNavMeshQuery
// built from the same mmaps files (AgentNavRoute.h): it shares nothing with the maps' meshes,
// which map threads add tiles to as grids load and whose queries are per map and not thread
// safe. Finished routes are handed back through a queue and sent from WorldScript::OnUpdate on
// the world thread. So a query can never stall the world, however long it searches; the limits
// below bound the queue, the memory and each search (the node budget).
//
// It also tells a character where services are on its map (AgentNavServices.h): the spawn table
// is read from what ObjectMgr loaded, once, on the world thread at startup, and handed to the
// search thread, which answers from it; no database query per request. The faction check uses
// the faction templates alone (reputation is not looked at).

#include "AgentNavProtocol.h"
#include "AgentNavRoute.h"
#include "AgentNavServices.h"
#include "CreatureData.h"
#include "DBCStores.h"
#include "ObjectMgr.h"
#include "SpellInfo.h"
#include "SpellMgr.h"
#include "Trainer.h"
#include "UnitDefines.h"
#include "Chat.h"
#include "Config.h"
#include "Log.h"
#include "MapDefines.h"
#include "ObjectAccessor.h"
#include "ObjectGuid.h"
#include "Player.h"
#include "PlayerScript.h"
#include "ScriptMgr.h"
#include "SharedDefines.h"
#include "World.h"
#include "WorldPacket.h"
#include "WorldScript.h"

#include <chrono>
#include <condition_variable>
#include <deque>
#include <map>
#include <memory>
#include <mutex>
#include <string>
#include <thread>
#include <unordered_map>
#include <vector>

static_assert(AgentNav::kMmapMagic == MMAP_MAGIC, "mmtile magic");
static_assert(AgentNav::kMmapVersion == MMAP_VERSION, "mmtile version: the route search reads this core's tiles");
static_assert(AgentNav::kMmapHeaderSize == sizeof(MmapTileHeader), "mmtile header");
static_assert(AgentNav::kFlagQuestGiver == UNIT_NPC_FLAG_QUESTGIVER && AgentNav::kFlagTrainer == UNIT_NPC_FLAG_TRAINER &&
    AgentNav::kFlagRepair == UNIT_NPC_FLAG_REPAIR && AgentNav::kFlagFlightMaster == UNIT_NPC_FLAG_FLIGHTMASTER &&
    AgentNav::kFlagInnkeeper == UNIT_NPC_FLAG_INNKEEPER && AgentNav::kFlagBanker == UNIT_NPC_FLAG_BANKER &&
    AgentNav::kFlagVendors == (uint32(UNIT_NPC_FLAG_VENDOR_MASK) | uint32(UNIT_NPC_FLAG_VENDOR_FOOD)), "npc flags");
static_assert(AgentNav::kSkillCooking == SKILL_COOKING && AgentNav::kSkillFirstAid == SKILL_FIRST_AID, "skill lines");
static_assert(std::tuple_size_v<decltype(AgentNav::Faction::enemies)> == MAX_FACTION_RELATIONS, "faction relations");
static_assert(AgentNav::kGround == NAV_GROUND && AgentNav::kMagma == NAV_MAGMA && AgentNav::kSlime == NAV_SLIME &&
    AgentNav::kWater == NAV_WATER && AgentNav::kSteep == NAV_GROUND_STEEP, "terrain flags");

namespace
{
struct Settings
{
    bool enable = true;
    AgentNav::Limits limits;
    uint32 maxQueued = 64;
    uint32 maxQueuedPerCharacter = 2;
    uint32 maxLoadedTiles = 128;
    uint32 searchNodes = 65535;
    float legSnap = 40.0f;
    uint32 slowMs = 50;
};

struct Job
{
    ObjectGuid guid;
    uint32 map = 0;
    AgentNav::Message request;
    std::vector<AgentNav::Circle> avoid;
    AgentNav::RouteOptions options;
    // A services request (Verb::Services): who asks, from where; answered from the spawn table.
    AgentNav::ServiceQuery services;
};

struct Done
{
    ObjectGuid guid;
    std::vector<std::string> lines;
    uint32 ms = 0;
    uint32 map = 0;
};

struct Character
{
    AgentNav::RateLimiter rate;
    AgentNav::PendingAvoid avoid;
    uint32 queued = 0;
};

uint64 NowMs()
{
    return uint64(std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::steady_clock::now().time_since_epoch()).count());
}

// The search thread and its two queues.
class Service
{
public:
    // `spawns`: the service spawn table, read once on the world thread; the search thread owns it.
    void Start(std::string const& mmaps, Settings const& settings, std::vector<AgentNav::Spawn> spawns)
    {
        Stop();
        _stop = false;
        _thread = std::thread([this, mmaps, nodes = int(settings.searchNodes), tiles = std::size_t(settings.maxLoadedTiles),
                                  spawns = std::move(spawns)]
        {
            Run(mmaps, nodes, tiles, spawns);
        });
    }

    void Stop()
    {
        {
            std::lock_guard<std::mutex> guard(_lock);
            _stop = true;
        }
        _wake.notify_all();
        if (_thread.joinable())
            _thread.join();
        std::lock_guard<std::mutex> guard(_lock);
        _jobs.clear();
        _done.clear();
    }

    bool Running() const { return _thread.joinable(); }

    // False when the queue is full.
    bool Push(Job&& job, uint32 maxQueued)
    {
        {
            std::lock_guard<std::mutex> guard(_lock);
            if (_jobs.size() >= maxQueued)
                return false;
            _jobs.push_back(std::move(job));
        }
        _wake.notify_one();
        return true;
    }

    std::deque<Done> TakeDone()
    {
        std::deque<Done> out;
        std::lock_guard<std::mutex> guard(_lock);
        out.swap(_done);
        return out;
    }

private:
    void Run(std::string const& mmaps, int nodes, std::size_t tiles, std::vector<AgentNav::Spawn> const& spawns)
    {
        AgentNav::MeshCache cache(mmaps, nodes, tiles);
        while (true)
        {
            Job job;
            {
                std::unique_lock<std::mutex> guard(_lock);
                _wake.wait(guard, [this] { return _stop || !_jobs.empty(); });
                if (_stop)
                    return;
                job = std::move(_jobs.front());
                _jobs.pop_front();
            }
            uint64 const began = NowMs();
            Done done;
            done.guid = job.guid;
            done.map = job.map;
            if (job.request.verb == AgentNav::Verb::Services)
            {
                done.lines = AgentNav::ServiceReply(job.request.id, AgentNav::SelectServices(spawns, job.services),
                    AgentNav::kMaxPayload);
                done.ms = uint32(NowMs() - began);
                std::lock_guard<std::mutex> guard(_lock);
                _done.push_back(std::move(done));
                continue;
            }
            AgentNav::RouteResult const route = cache.Route(job.map, job.request.from, job.request.to, job.avoid, job.options);
            if (!route.error.empty())
                done.lines.push_back(AgentNav::ErrorReply(job.request.id, route.error,
                    route.error == "nomesh" ? "this realm has no navmesh for this map" : "start or end is not on walkable ground"));
            else
                done.lines = AgentNav::RouteReply(job.request.id, route.flags, route.points);
            done.ms = uint32(NowMs() - began);
            std::lock_guard<std::mutex> guard(_lock);
            _done.push_back(std::move(done));
        }
    }

    std::mutex _lock;
    std::condition_variable _wake;
    std::deque<Job> _jobs;
    std::deque<Done> _done;
    std::thread _thread;
    bool _stop = false;
};

std::mutex gLock;
Settings gSettings;
std::unordered_map<ObjectGuid, Character> gCharacters;
Service gService;

Settings Current()
{
    std::lock_guard<std::mutex> guard(gLock);
    return gSettings;
}

void Send(Player* player, std::string const& payload)
{
    std::string wire(AgentNav::kPrefix);
    wire += '\t';
    wire += payload;
    WorldPacket data;
    ChatHandler::BuildChatPacket(data, CHAT_MSG_WHISPER, LANG_ADDON, player, nullptr, wire);
    player->SendDirectMessage(&data);
}

// A faction template's reaction fields, as AgentNavServices.h compares them.
AgentNav::Faction FactionOf(FactionTemplateEntry const* entry)
{
    AgentNav::Faction out;
    if (!entry)
        return out;
    out.faction = entry->faction;
    out.ourMask = entry->ourMask;
    out.friendlyMask = entry->friendlyMask;
    out.hostileMask = entry->hostileMask;
    for (std::size_t i = 0; i < out.enemies.size(); ++i)
    {
        out.enemies[i] = entry->enemyFaction[i];
        out.friends[i] = entry->friendFaction[i];
    }
    return out;
}

// A services request as the search thread answers it: who asks, where, and what.
AgentNav::ServiceQuery Asking(Player* player, AgentNav::Message const& message, AgentNav::Limits const& limits)
{
    AgentNav::ServiceQuery q;
    q.kinds = message.kinds;
    q.map = player->GetMapId();
    q.phaseMask = player->GetPhaseMask();
    q.x = player->GetPositionX();
    q.y = player->GetPositionY();
    q.radius = message.radius > 0.0f ? std::min(message.radius, limits.serviceRadius) : limits.serviceRadius;
    q.playerClass = player->getClass();
    q.faction = FactionOf(player->GetFactionTemplateEntry());
    q.cap = limits.maxServices;
    return q;
}

// The skill lines a spell steps or sets (SPELL_EFFECT_SKILL_STEP, SPELL_EFFECT_SKILL), and those of
// the spells it teaches (SPELL_EFFECT_LEARN_SPELL), one level down.
void SkillsOf(uint32 spellId, std::map<uint32, uint32>& votes, bool teach)
{
    SpellInfo const* info = sSpellMgr->GetSpellInfo(spellId);
    if (!info)
        return;
    for (SpellEffectInfo const& effect : info->GetEffects())
    {
        if ((effect.Effect == SPELL_EFFECT_SKILL_STEP || effect.Effect == SPELL_EFFECT_SKILL) && effect.MiscValue > 0)
            ++votes[uint32(effect.MiscValue)];
        else if (teach && effect.Effect == SPELL_EFFECT_LEARN_SPELL && effect.TriggerSpell)
            SkillsOf(effect.TriggerSpell, votes, false);
    }
}

// The profession a tradeskill trainer teaches, and its name. AzerothCore's `trainer` rows of type
// Tradeskill carry no requirement (0) and serve weapon masters too, so it is read off the trainer's
// spells: the skill line they require (ReqSkillLine, set on every rank and recipe past the first)
// and the skill lines they step (the "Apprentice X" spells), the most common one that is a
// profession or secondary skill (SkillLine.dbc categories 11 and 9). 0 for none.
uint32 ProfessionOf(Trainer::Trainer const& trainer, std::string& name)
{
    std::map<uint32, uint32> votes;
    for (Trainer::Spell const& spell : trainer.GetSpells())
    {
        if (spell.ReqSkillLine)
            ++votes[spell.ReqSkillLine];
        SkillsOf(spell.SpellId, votes, true);
    }
    uint32 best = 0, most = 0;
    for (auto const& [skill, count] : votes)
    {
        SkillLineEntry const* line = sSkillLineStore.LookupEntry(skill);
        if (!line || (line->categoryId != SKILL_CATEGORY_PROFESSION && line->categoryId != SKILL_CATEGORY_SECONDARY))
            continue;
        if (count > most)
        {
            best = skill;
            most = count;
        }
    }
    if (SkillLineEntry const* line = best ? sSkillLineStore.LookupEntry(best) : nullptr)
        name = line->name[LOCALE_enUS] ? line->name[LOCALE_enUS] : "";
    return best;
}

// The creature spawns that offer a service, from the world database ObjectMgr has loaded: the
// spawn's own NPC flags or its template's (ObjectMgr::ChooseCreatureFlags), the template's faction
// and name, and the class its default trainer teaches. Read once, on the world thread at startup.
std::vector<AgentNav::Spawn> ServiceSpawns()
{
    std::vector<AgentNav::Spawn> out;
    for (auto const& [spawnId, data] : sObjectMgr->GetAllCreatureData())
    {
        CreatureTemplate const* info = sObjectMgr->GetCreatureTemplate(data.id);
        if (!info)
            continue;
        uint32 npcflag = 0, unitFlags = 0, dynamicFlags = 0;
        ObjectMgr::ChooseCreatureFlags(info, npcflag, unitFlags, dynamicFlags, &data);
        if (!(npcflag & AgentNav::kServiceFlags))
            continue;
        AgentNav::Spawn s;
        s.entry = data.id;
        s.map = data.mapid;
        s.phaseMask = data.phaseMask;
        s.x = data.posX;
        s.y = data.posY;
        s.z = data.posZ;
        s.npcFlags = npcflag;
        if (Trainer::Trainer const* trainer = sObjectMgr->GetTrainer(data.id))
        {
            if (trainer->GetTrainerType() == Trainer::Type::Class)
                s.trainerClass = uint8(trainer->GetTrainerRequirement());
            else if (trainer->GetTrainerType() == Trainer::Type::Tradeskill)
                s.profession = ProfessionOf(*trainer, s.professionName);
        }
        s.faction = FactionOf(sFactionTemplateStore.LookupEntry(info->faction));
        s.name = info->Name;
        out.push_back(std::move(s));
    }
    return out;
}

// One client line. Every line gets an answer, a refusal included.
void Handle(Player* player, std::string_view payload)
{
    Settings const settings = Current();
    AgentNav::Message message;
    std::string const wrong = AgentNav::Parse(payload, message);
    if (!wrong.empty())
    {
        Send(player, AgentNav::ErrorReply(message.id, "bad", wrong));
        return;
    }
    if (message.verb == AgentNav::Verb::Hello)
    {
        Send(player, AgentNav::HelloReply(settings.limits));
        return;
    }
    std::string refusal;
    std::vector<AgentNav::Circle> avoid;
    {
        std::lock_guard<std::mutex> guard(gLock);
        Character& character = gCharacters[player->GetGUID()];
        if (message.verb == AgentNav::Verb::Avoid)
        {
            std::string const why = character.avoid.Add(message, settings.limits.maxAvoid);
            if (!why.empty())
                refusal = AgentNav::ErrorReply(message.id, "bad", why);
        }
        else
        {
            avoid = character.avoid.Take(message.id);
            switch (character.rate.Allow(NowMs(), settings.limits.perSecond, settings.limits.perMinute))
            {
                case AgentNav::RateLimiter::Verdict::PerSecond:
                    refusal = AgentNav::ErrorReply(message.id, "rate", std::to_string(settings.limits.perSecond) + " a second");
                    break;
                case AgentNav::RateLimiter::Verdict::PerMinute:
                    refusal = AgentNav::ErrorReply(message.id, "rate", std::to_string(settings.limits.perMinute) + " a minute");
                    break;
                case AgentNav::RateLimiter::Verdict::Ok:
                    if (character.queued >= settings.maxQueuedPerCharacter)
                        refusal = AgentNav::ErrorReply(message.id, "busy", "wait for the answers already asked for");
                    break;
            }
        }
    }
    if (message.verb == AgentNav::Verb::Avoid)
    {
        // Circles are answered only when refused: the route's own reply covers the rest.
        if (!refusal.empty())
            Send(player, refusal);
        return;
    }
    bool const services = message.verb == AgentNav::Verb::Services;
    if (refusal.empty() && !services)
        refusal = AgentNav::CheckPath(message, settings.limits,
            {player->GetPositionX(), player->GetPositionY(), player->GetPositionZ()});
    if (refusal.empty() && !gService.Running())
        refusal = AgentNav::ErrorReply(message.id, "nomesh", "this realm has no navmesh");
    if (!refusal.empty())
    {
        Send(player, refusal);
        return;
    }
    Job job;
    job.guid = player->GetGUID();
    job.map = player->GetMapId();
    job.request = message;
    job.avoid = std::move(avoid);
    job.options.maxPoints = int(settings.limits.maxWaypoints);
    job.options.endSnap = message.near ? settings.legSnap : 0.0f;
    job.options.anyHeight = message.anyHeight;
    if (services)
        job.services = Asking(player, message, settings.limits);
    {
        std::lock_guard<std::mutex> guard(gLock);
        ++gCharacters[player->GetGUID()].queued;
    }
    if (gService.Push(std::move(job), settings.maxQueued))
        return;
    {
        std::lock_guard<std::mutex> guard(gLock);
        --gCharacters[player->GetGUID()].queued;
    }
    Send(player, AgentNav::ErrorReply(message.id, "busy", "the realm's request queue is full"));
}

class AgentNavWorld : public WorldScript
{
public:
    AgentNavWorld()
        : WorldScript("AgentNavWorld",
              {WORLDHOOK_ON_AFTER_CONFIG_LOAD, WORLDHOOK_ON_STARTUP, WORLDHOOK_ON_UPDATE, WORLDHOOK_ON_SHUTDOWN})
    {
    }

    void OnAfterConfigLoad(bool /*reload*/) override
    {
        Settings s;
        s.enable = sConfigMgr->GetOption<bool>("AgentNav.Enable", true);
        s.limits.perSecond = sConfigMgr->GetOption<uint32>("AgentNav.QueriesPerSecond", 5);
        s.limits.perMinute = sConfigMgr->GetOption<uint32>("AgentNav.QueriesPerMinute", 240);
        s.limits.maxDistance = sConfigMgr->GetOption<float>("AgentNav.MaxDistance", 1500.0f);
        s.limits.maxStartDistance = sConfigMgr->GetOption<float>("AgentNav.MaxStartDistance", 100.0f);
        s.limits.maxWaypoints = std::clamp<uint32>(sConfigMgr->GetOption<uint32>("AgentNav.MaxWaypoints", 256), 2, 512);
        // The client's own filter holds 64 circles.
        s.limits.maxAvoid = std::min<uint32>(sConfigMgr->GetOption<uint32>("AgentNav.MaxAvoidCircles", 64), 64);
        s.maxQueued = sConfigMgr->GetOption<uint32>("AgentNav.MaxQueued", 64);
        s.maxQueuedPerCharacter = std::max<uint32>(sConfigMgr->GetOption<uint32>("AgentNav.MaxQueuedPerCharacter", 2), 1);
        // The search thread reads these two once, when it starts: a restart applies a change.
        s.maxLoadedTiles = sConfigMgr->GetOption<uint32>("AgentNav.MaxLoadedTiles", 128);
        s.searchNodes = sConfigMgr->GetOption<uint32>("AgentNav.SearchNodes", 65535);
        s.legSnap = sConfigMgr->GetOption<float>("AgentNav.LegSnap", 40.0f);
        s.slowMs = sConfigMgr->GetOption<uint32>("AgentNav.SlowQueryMs", 50);
        s.limits.serviceRadius = sConfigMgr->GetOption<float>("AgentNav.ServiceRadius", 2000.0f);
        s.limits.maxServices = std::clamp<uint32>(sConfigMgr->GetOption<uint32>("AgentNav.MaxServices", 20), 1, 100);
        std::lock_guard<std::mutex> guard(gLock);
        gSettings = s;
    }

    void OnStartup() override
    {
        Settings const settings = Current();
        if (!settings.enable)
        {
            LOG_INFO("server.loading", ">> mod-agent-nav: disabled");
            return;
        }
        std::string const mmaps = sWorld->GetDataPath() + "mmaps";
        std::vector<AgentNav::Spawn> spawns = ServiceSpawns();
        std::size_t const places = spawns.size();
        gService.Start(mmaps, settings, std::move(spawns));
        LOG_INFO("server.loading",
            ">> mod-agent-nav: answering route requests from {} ({} a second and {} a minute per character, {} yd, {} corners); {} service spawns",
            mmaps, settings.limits.perSecond, settings.limits.perMinute, uint32(settings.limits.maxDistance),
            settings.limits.maxWaypoints, places);
    }

    void OnShutdown() override { gService.Stop(); }

    void OnUpdate(uint32 /*diff*/) override
    {
        if (!gService.Running())
            return;
        std::deque<Done> done = gService.TakeDone();
        if (done.empty())
            return;
        uint32 const slowMs = Current().slowMs;
        for (Done const& reply : done)
        {
            {
                std::lock_guard<std::mutex> guard(gLock);
                auto found = gCharacters.find(reply.guid);
                if (found != gCharacters.end() && found->second.queued > 0)
                    --found->second.queued;
            }
            if (slowMs != 0 && reply.ms >= slowMs)
                LOG_WARN("module", "mod-agent-nav: a route on map {} for {} took {} ms", reply.map, reply.guid.ToString(), reply.ms);
            // Logged out meanwhile: nobody to tell.
            Player* player = ObjectAccessor::FindConnectedPlayer(reply.guid);
            if (!player || !player->GetSession())
                continue;
            for (std::string const& line : reply.lines)
                Send(player, line);
        }
    }
};

class AgentNavPlayer : public PlayerScript
{
public:
    AgentNavPlayer() : PlayerScript("AgentNavPlayer", {PLAYERHOOK_CAN_PLAYER_USE_PRIVATE_CHAT, PLAYERHOOK_ON_LOGOUT}) { }

    using PlayerScript::OnPlayerCanUseChat;

    // A LANG_ADDON whisper to oneself with our prefix is a request: answer it and deliver
    // nothing. Anything else is not ours.
    bool OnPlayerCanUseChat(Player* player, uint32 type, uint32 language, std::string& msg, Player* receiver) override
    {
        if (!player || receiver != player || type != CHAT_MSG_WHISPER || language != uint32(LANG_ADDON))
            return true;
        std::size_t const envelope = AgentNav::kPrefix.size() + 1;
        if (msg.size() < envelope || msg.compare(0, AgentNav::kPrefix.size(), AgentNav::kPrefix) != 0 ||
            msg[AgentNav::kPrefix.size()] != '\t')
            return true;
        // Disabled: deliver the whisper back like any other, which tells the client there is no
        // route service here.
        if (!Current().enable)
            return true;
        Handle(player, std::string_view(msg).substr(envelope));
        return false;
    }

    void OnPlayerLogout(Player* player) override
    {
        std::lock_guard<std::mutex> guard(gLock);
        gCharacters.erase(player->GetGUID());
    }
};
}

void Addmod_agent_navScripts()
{
    new AgentNavWorld();
    new AgentNavPlayer();
}
