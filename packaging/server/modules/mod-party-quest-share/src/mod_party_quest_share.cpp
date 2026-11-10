// mod-party-quest-share: playerbots in a party accept a quest a party member shares.
//
// The core's share handler (WorldSession::HandlePushQuestToParty) checks every party member
// (quest status, CanTakeQuest, quest log space, not already answering another share), marks the
// sharer as the member's "divider" and sends the member the quest details, to accept or decline.
// A playerbot has no client to answer. mod-playerbots answers for a bot only when the sharer is
// that bot's master, so a bot whose master is someone else (an invited random bot's master is
// the party leader) never answers, and stays "busy" for every later share.
//
// This script answers for the bot at the moment the core offers it the quest: the details packet
// queued for a client-less session, whose quest giver is the bot itself (as the share handler
// sends it) while its divider is a member of its own party. It then runs the core's own accept
// handler on the bot's session, as a client pressing Accept would, so the accept rules are the
// core's and not a copy of them. Real players have a socket and are never touched.
//
// Cost: one opcode comparison per packet sent. The accept itself runs inside the sharer's
// CMSG_PUSHQUESTTOPARTY handler (world thread), once per eligible bot per share.

#include "Config.h"
#include "Group.h"
#include "Log.h"
#include "ObjectAccessor.h"
#include "Opcodes.h"
#include "Player.h"
#include "QuestDef.h"
#include "ScriptMgr.h"
#include "ServerScript.h"
#include "WorldPacket.h"
#include "WorldScript.h"
#include "WorldSession.h"

#include <atomic>

namespace
{
// Read wherever a packet is sent, so not only on the world thread.
std::atomic<bool> gEnable{true};

// SMSG_QUESTGIVER_QUEST_DETAILS starts: quest giver guid, divider guid, quest id.
constexpr std::size_t QUEST_GIVER_OFFSET = 0;
constexpr std::size_t QUEST_ID_OFFSET = 16;
constexpr std::size_t DETAILS_HEADER_SIZE = 20;

class PartyQuestShareWorld : public WorldScript
{
public:
    PartyQuestShareWorld() : WorldScript("PartyQuestShareWorld", {WORLDHOOK_ON_AFTER_CONFIG_LOAD, WORLDHOOK_ON_STARTUP}) { }

    void OnAfterConfigLoad(bool /*reload*/) override
    {
        gEnable = sConfigMgr->GetOption<bool>("PartyQuestShare.Enable", true);
    }

    void OnStartup() override
    {
        LOG_INFO("server.loading", ">> mod-party-quest-share: party bots {} shared quests",
            gEnable ? "accept" : "are not answered for on");
    }
};

class PartyQuestShareServer : public ServerScript
{
public:
    PartyQuestShareServer() : ServerScript("PartyQuestShareServer", {SERVERHOOK_ON_PACKET_SENT}) { }

    void OnPacketSent(WorldSession* session, WorldPacket const& packet) override
    {
        if (packet.GetOpcode() != SMSG_QUESTGIVER_QUEST_DETAILS || !gEnable)
            return;

        // Only sessions without a client: a real player answers the dialog.
        if (!session || !session->IsHeadless() || packet.size() < DETAILS_HEADER_SIZE)
            return;

        Player* bot = session->GetPlayer();
        if (!bot || !bot->IsInWorld())
            return;

        // A share, not a quest giver the bot is talking to: the share handler names the receiver
        // itself as the quest giver, and has just set the sharer as its divider.
        ObjectGuid sharerGuid = bot->GetDivider();
        if (!sharerGuid || packet.read<uint64>(QUEST_GIVER_OFFSET) != bot->GetGUID().GetRawValue())
            return;

        // The core shares only within one map; anything else is a divider left over from before.
        Player* sharer = ObjectAccessor::GetPlayer(*bot, sharerGuid);
        Group* group = bot->GetGroup();
        if (!sharer || sharer == bot || !group || sharer->GetGroup() != group)
            return;

        uint32 questId = packet.read<uint32>(QUEST_ID_OFFSET);

        // What a client sends on Accept. The handler checks again that the sharer can share the
        // quest and the bot can take it, tells the sharer "accepted", clears the divider and adds
        // the quest (with its source item and spell).
        WorldPacket accept(CMSG_QUESTGIVER_ACCEPT_QUEST, 8 + 4 + 4);
        accept << sharerGuid << questId << uint32(0);
        session->HandleQuestgiverAcceptQuestOpcode(accept);

        // The handler leaves the offer open where a client could still answer later (bot or
        // sharer dead, a quest whose source item sells). Nobody will: decline, so the bot is
        // not "busy" for the next share.
        if (bot->GetDivider())
        {
            sharer->SendPushToPartyResponse(bot, QUEST_PARTY_MSG_DECLINE_QUEST);
            bot->SetDivider();
        }

        LOG_DEBUG("module", "mod-party-quest-share: {} answered quest {} shared by {}: {}",
            bot->GetName(), questId, sharer->GetName(),
            bot->GetQuestStatus(questId) != QUEST_STATUS_NONE ? "has it" : "does not have it");
    }
};
}

void Addmod_party_quest_shareScripts()
{
    new PartyQuestShareWorld();
    new PartyQuestShareServer();
}
