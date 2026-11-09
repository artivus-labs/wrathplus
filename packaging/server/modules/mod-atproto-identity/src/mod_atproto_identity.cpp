// mod-atproto-identity: game accounts linked to ATProto identities (WrathPlus open beta).
//
// The WrathPlus account service (services/account) proves who someone is (OAuth or an app
// password), creates their game account, and then tells the realm over SOAP:
//
//   .atproto link <ACCOUNT> <did> <handle>   console only: remember who owns the account
//   .atproto unlink <ACCOUNT>                console only
//   .atproto events [limit]                  console only: queued progress, one "EV ..." line each
//   .atproto ack <id>                        console only: forget events up to and including id
//   .atproto whoami                          players: which identity this account belongs to
//
// Links live in the auth database (account_atproto) and in memory; a linked player is greeted
// "Signed in as @handle" on entering the world. Level-ups, quest completions and achievements of
// linked characters are queued in memory and written to the characters database asynchronously
// (atproto_event), so the service can fetch them and publish them as ATProto records. Nothing on
// the world or map threads waits on the database: only startup loads, and `.atproto link`
// resolves the account name the way the core's own `.account` commands do.
//
// SOAP rather than a direct database connection: the service needs no MySQL credentials (MySQL
// stays inside the Docker network), every write is validated here, and the realm owns its schema.

#include "AccountMgr.h"
#include "AchievementMgr.h"
#include "Chat.h"
#include "ChatCommand.h"
#include "CommandScript.h"
#include "Config.h"
#include "DatabaseEnv.h"
#include "Log.h"
#include "Player.h"
#include "PlayerScript.h"
#include "QuestDef.h"
#include "ScriptMgr.h"
#include "Tokenize.h"
#include "World.h"
#include "WorldScript.h"
#include "WorldSession.h"

#include <algorithm>
#include <cctype>
#include <ctime>
#include <deque>
#include <mutex>
#include <string>
#include <string_view>
#include <unordered_map>

using namespace Acore::ChatCommands;

namespace
{
struct Link
{
    std::string did;
    std::string handle;
};

struct Event
{
    uint64 id = 0;
    uint32 at = 0;
    std::string did;
    uint32 guid = 0;
    std::string kind;
    uint32 value = 0;
    uint8 level = 0;
    uint8 race = 0;
    uint8 cls = 0;
    uint8 gender = 0;
    std::string name;
    std::string title;
};

struct Settings
{
    bool enable = true;
    bool welcome = true;
    bool quests = true;
    uint32 maxQueued = 20000;
};

// Hooks run on map update threads as well as the world thread; one lock guards all of it.
std::mutex gLock;
Settings gSettings;
std::unordered_map<uint32, Link> gLinks;
std::deque<Event> gQueue;
uint64 gNextId = 1;
bool gDropWarned = false;

// --- validation (the account service validates too; this side never trusts it) ------------------

bool IsLowerAlnum(char c) { return (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9'); }

bool ValidHandle(std::string_view h)
{
    if (h.size() < 3 || h.size() > 253 || h.front() == '.' || h.back() == '.')
        return false;
    bool dot = false;
    std::size_t label = 0;
    char prev = '.';
    for (char c : h)
    {
        if (c == '.')
        {
            if (prev == '.' || prev == '-' || label == 0)
                return false;
            dot = true;
            label = 0;
        }
        else if (IsLowerAlnum(c) || c == '-')
        {
            if (c == '-' && prev == '.')
                return false;
            if (++label > 63)
                return false;
        }
        else
            return false;
        prev = c;
    }
    return dot && prev != '-';
}

bool ValidDid(std::string_view d)
{
    constexpr std::string_view plc = "did:plc:";
    constexpr std::string_view web = "did:web:";
    if (d.substr(0, plc.size()) == plc)
    {
        std::string_view id = d.substr(plc.size());
        return id.size() == 24 && std::all_of(id.begin(), id.end(), [](char c) { return (c >= 'a' && c <= 'z') || (c >= '2' && c <= '7'); });
    }
    if (d.substr(0, web.size()) == web)
        return ValidHandle(d.substr(web.size()));
    return false;
}

bool ValidAccountName(std::string_view n)
{
    return !n.empty() && n.size() <= 16 && std::all_of(n.begin(), n.end(), [](char c) { return (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9'); });
}

// Strings from game data (character names, quest titles) go into SQL as hex literals, so no
// quoting or escaping is involved, and onto the event line with whitespace flattened.
std::string Hex(std::string const& s)
{
    static char const* digits = "0123456789ABCDEF";
    std::string out = "X'";
    out.reserve(s.size() * 2 + 3);
    for (unsigned char c : s)
    {
        out += digits[c >> 4];
        out += digits[c & 15];
    }
    out += '\'';
    return out;
}

std::string OneLine(std::string s, std::size_t max)
{
    for (char& c : s)
        if (c == '\n' || c == '\r' || c == '\t' || c == '\0')
            c = ' ';
    if (s.size() > max)
        s.resize(max);
    return s;
}

// --- links ---------------------------------------------------------------------------------------

bool LinkFor(uint32 accountId, Link& out)
{
    std::lock_guard<std::mutex> guard(gLock);
    auto it = gLinks.find(accountId);
    if (it == gLinks.end())
        return false;
    out = it->second;
    return true;
}

void LoadState()
{
    std::lock_guard<std::mutex> guard(gLock);
    gLinks.clear();
    gQueue.clear();
    if (QueryResult result = LoginDatabase.Query("SELECT `account_id`, `did`, `handle` FROM `account_atproto`"))
    {
        do
        {
            Field* f = result->Fetch();
            gLinks[f[0].Get<uint32>()] = Link{f[1].Get<std::string>(), f[2].Get<std::string>()};
        } while (result->NextRow());
    }
    gNextId = 1;
    if (QueryResult result = CharacterDatabase.Query(
            "SELECT `id`, `occurred_at`, `did`, `guid`, `kind`, `value`, `level`, `race`, `class`, `gender`, `name`, `title` "
            "FROM `atproto_event` ORDER BY `id`"))
    {
        do
        {
            Field* f = result->Fetch();
            Event e;
            e.id = f[0].Get<uint64>();
            e.at = f[1].Get<uint32>();
            e.did = f[2].Get<std::string>();
            e.guid = f[3].Get<uint32>();
            e.kind = f[4].Get<std::string>();
            e.value = f[5].Get<uint32>();
            e.level = f[6].Get<uint8>();
            e.race = f[7].Get<uint8>();
            e.cls = f[8].Get<uint8>();
            e.gender = f[9].Get<uint8>();
            e.name = f[10].Get<std::string>();
            e.title = f[11].Get<std::string>();
            gNextId = std::max(gNextId, e.id + 1);
            gQueue.push_back(std::move(e));
        } while (result->NextRow());
    }
    LOG_INFO("module", ">> mod-atproto-identity: {} linked accounts, {} queued events", gLinks.size(), gQueue.size());
}

// --- events --------------------------------------------------------------------------------------

void Emit(Player* player, char const* kind, uint32 value, std::string title)
{
    if (!gSettings.enable || !player || !player->GetSession())
        return;
    Link link;
    if (!LinkFor(player->GetSession()->GetAccountId(), link))
        return; // bots and unlinked accounts: nothing to publish

    Event e;
    e.at = uint32(std::time(nullptr));
    e.did = link.did;
    e.guid = player->GetGUID().GetCounter();
    e.kind = kind;
    e.value = value;
    e.level = player->GetLevel();
    e.race = player->getRace();
    e.cls = player->getClass();
    e.gender = player->getGender();
    e.name = OneLine(player->GetName(), 48);
    e.title = OneLine(std::move(title), 512);

    {
        std::lock_guard<std::mutex> guard(gLock);
        if (gQueue.size() >= gSettings.maxQueued)
        {
            if (!gDropWarned)
                LOG_WARN("module", "mod-atproto-identity: {} events queued and unacknowledged; dropping new ones (is the account service running?)", gQueue.size());
            gDropWarned = true;
            return;
        }
        e.id = gNextId++;
        gQueue.push_back(e);
    }
    CharacterDatabase.Execute(
        "INSERT INTO `atproto_event` (`id`, `occurred_at`, `did`, `guid`, `kind`, `value`, `level`, `race`, `class`, `gender`, `name`, `title`) "
        "VALUES ({}, {}, '{}', {}, '{}', {}, {}, {}, {}, {}, {}, {})",
        e.id, e.at, e.did, e.guid, e.kind, e.value, uint32(e.level), uint32(e.race), uint32(e.cls), uint32(e.gender), Hex(e.name), Hex(e.title));
}

class AtprotoIdentityWorld : public WorldScript
{
public:
    AtprotoIdentityWorld() : WorldScript("AtprotoIdentityWorld", {WORLDHOOK_ON_AFTER_CONFIG_LOAD, WORLDHOOK_ON_STARTUP}) { }

    void OnAfterConfigLoad(bool /*reload*/) override
    {
        std::lock_guard<std::mutex> guard(gLock);
        gSettings.enable = sConfigMgr->GetOption<bool>("AtprotoIdentity.Enable", true);
        gSettings.welcome = sConfigMgr->GetOption<bool>("AtprotoIdentity.Welcome", true);
        gSettings.quests = sConfigMgr->GetOption<bool>("AtprotoIdentity.Events.Quests", true);
        gSettings.maxQueued = sConfigMgr->GetOption<uint32>("AtprotoIdentity.Events.MaxQueued", 20000);
    }

    void OnStartup() override
    {
        if (gSettings.enable)
            LoadState();
    }
};

class AtprotoIdentityPlayer : public PlayerScript
{
public:
    AtprotoIdentityPlayer()
        : PlayerScript("AtprotoIdentityPlayer",
              {PLAYERHOOK_ON_LOGIN, PLAYERHOOK_ON_LEVEL_CHANGED, PLAYERHOOK_ON_PLAYER_COMPLETE_QUEST, PLAYERHOOK_ON_ACHI_COMPLETE})
    {
    }

    void OnPlayerLogin(Player* player) override
    {
        if (!gSettings.enable)
            return;
        Link link;
        if (!LinkFor(player->GetSession()->GetAccountId(), link))
            return;
        if (gSettings.welcome)
            ChatHandler(player->GetSession()).PSendSysMessage("Signed in as @{}", link.handle);
        Emit(player, "login", 0, "");
    }

    void OnPlayerLevelChanged(Player* player, uint8 oldLevel) override
    {
        if (player->GetLevel() > oldLevel)
            Emit(player, "level", player->GetLevel(), "");
    }

    void OnPlayerCompleteQuest(Player* player, Quest const* quest) override
    {
        if (gSettings.quests && quest)
            Emit(player, "quest", quest->GetQuestId(), quest->GetTitle());
    }

    void OnPlayerAchievementComplete(Player* player, AchievementEntry const* achievement) override
    {
        if (!achievement)
            return;
        char const* name = achievement->name[sWorld->GetDefaultDbcLocale()];
        Emit(player, "achievement", achievement->ID, name ? name : "");
    }
};

// --- commands ------------------------------------------------------------------------------------

std::vector<std::string_view> Args(char const* args)
{
    return Acore::Tokenize(args ? std::string_view(args) : std::string_view(), ' ', false);
}

bool ParseUInt(std::string_view s, uint64& out)
{
    if (s.empty() || s.size() > 19 || !std::all_of(s.begin(), s.end(), [](char c) { return c >= '0' && c <= '9'; }))
        return false;
    out = std::stoull(std::string(s));
    return true;
}

class AtprotoIdentityCommands : public CommandScript
{
public:
    AtprotoIdentityCommands() : CommandScript("AtprotoIdentityCommands") { }

    ChatCommandTable GetCommands() const override
    {
        // SEC_CONSOLE: no in-game account reaches these; SOAP and the console run as console.
        static ChatCommandTable atprotoTable = {
            {"link", HandleLink, SEC_CONSOLE, Console::Yes},
            {"unlink", HandleUnlink, SEC_CONSOLE, Console::Yes},
            {"events", HandleEvents, SEC_CONSOLE, Console::Yes},
            {"ack", HandleAck, SEC_CONSOLE, Console::Yes},
            {"whoami", HandleWhoami, SEC_PLAYER, Console::No},
        };
        static ChatCommandTable commandTable = {
            {"atproto", atprotoTable},
        };
        return commandTable;
    }

    static bool HandleLink(ChatHandler* handler, char const* args)
    {
        auto a = Args(args);
        if (a.size() != 3 || !ValidAccountName(a[0]) || !ValidDid(a[1]) || !ValidHandle(a[2]))
        {
            handler->SendErrorMessage("Usage: .atproto link ACCOUNT did:plc:... handle.example.com", false);
            return false;
        }
        std::string name(a[0]);
        std::string did(a[1]);
        std::string handle(a[2]);
        uint32 accountId = AccountMgr::GetId(name);
        if (!accountId)
        {
            handler->SendErrorMessage("Account {} does not exist", name);
            return false;
        }
        // One account per DID: REPLACE drops any other account's row for this DID (unique key).
        LoginDatabase.Execute("REPLACE INTO `account_atproto` (`account_id`, `did`, `handle`) VALUES ({}, '{}', '{}')", accountId, did, handle);
        {
            std::lock_guard<std::mutex> guard(gLock);
            for (auto it = gLinks.begin(); it != gLinks.end();)
                it = (it->second.did == did && it->first != accountId) ? gLinks.erase(it) : std::next(it);
            gLinks[accountId] = Link{did, handle};
        }
        handler->PSendSysMessage("Linked {} to {} (@{})", name, did, handle);
        return true;
    }

    static bool HandleUnlink(ChatHandler* handler, char const* args)
    {
        auto a = Args(args);
        if (a.size() != 1 || !ValidAccountName(a[0]))
        {
            handler->SendErrorMessage("Usage: .atproto unlink ACCOUNT", false);
            return false;
        }
        std::string name(a[0]);
        uint32 accountId = AccountMgr::GetId(name);
        if (!accountId)
        {
            handler->SendErrorMessage("Account {} does not exist", name);
            return false;
        }
        LoginDatabase.Execute("DELETE FROM `account_atproto` WHERE `account_id` = {}", accountId);
        {
            std::lock_guard<std::mutex> guard(gLock);
            gLinks.erase(accountId);
        }
        handler->PSendSysMessage("Unlinked {}", name);
        return true;
    }

    // EV id unix did guid kind value level race class gender name [title...]
    static bool HandleEvents(ChatHandler* handler, char const* args)
    {
        auto a = Args(args);
        uint64 limit = 50;
        if (!a.empty() && (!ParseUInt(a[0], limit) || limit == 0 || limit > 500))
        {
            handler->SendErrorMessage("Usage: .atproto events [1-500]", false);
            return false;
        }
        std::vector<Event> out;
        {
            std::lock_guard<std::mutex> guard(gLock);
            for (auto const& e : gQueue)
            {
                if (out.size() >= limit)
                    break;
                out.push_back(e);
            }
        }
        for (auto const& e : out)
        {
            std::string line = Acore::StringFormat("EV {} {} {} {} {} {} {} {} {} {} {}", e.id, e.at, e.did, e.guid, e.kind, e.value,
                uint32(e.level), uint32(e.race), uint32(e.cls), uint32(e.gender), e.name);
            if (!e.title.empty())
                line += " " + e.title;
            handler->SendSysMessage(line);
        }
        if (out.empty())
            handler->SendSysMessage("No events");
        return true;
    }

    static bool HandleAck(ChatHandler* handler, char const* args)
    {
        auto a = Args(args);
        uint64 upTo = 0;
        if (a.size() != 1 || !ParseUInt(a[0], upTo))
        {
            handler->SendErrorMessage("Usage: .atproto ack ID", false);
            return false;
        }
        std::size_t removed = 0;
        {
            std::lock_guard<std::mutex> guard(gLock);
            while (!gQueue.empty() && gQueue.front().id <= upTo)
            {
                gQueue.pop_front();
                ++removed;
            }
            gDropWarned = false;
        }
        CharacterDatabase.Execute("DELETE FROM `atproto_event` WHERE `id` <= {}", upTo);
        handler->PSendSysMessage("Acknowledged {} events", removed);
        return true;
    }

    static bool HandleWhoami(ChatHandler* handler, char const* /*args*/)
    {
        WorldSession* session = handler->GetSession();
        if (!session)
            return false;
        Link link;
        if (!LinkFor(session->GetAccountId(), link))
        {
            handler->SendSysMessage("This account is not linked to an ATProto identity.");
            return true;
        }
        handler->PSendSysMessage("Signed in as @{} ({})", link.handle, link.did);
        return true;
    }
};
}

void Addmod_atproto_identityScripts()
{
    new AtprotoIdentityWorld();
    new AtprotoIdentityPlayer();
    new AtprotoIdentityCommands();
}
