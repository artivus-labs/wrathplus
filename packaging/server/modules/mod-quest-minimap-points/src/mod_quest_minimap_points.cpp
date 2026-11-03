// mod-quest-minimap-points: the quest points live in SQL (data/sql/db-world); this script only
// reports at startup how many quests now carry one, so a realm owner can see the module working.

#include "Log.h"
#include "ObjectMgr.h"
#include "QuestDef.h"
#include "ScriptMgr.h"

namespace
{
class QuestMinimapPointsWorld : public WorldScript
{
public:
    QuestMinimapPointsWorld() : WorldScript("QuestMinimapPointsWorld", {WORLDHOOK_ON_STARTUP}) { }

    void OnStartup() override
    {
        uint32 total = 0;
        uint32 pointed = 0;
        for (auto const& [id, quest] : sObjectMgr->GetQuestTemplates())
        {
            ++total;
            if (quest->GetPOIx() != 0.0f || quest->GetPOIy() != 0.0f)
                ++pointed;
        }
        LOG_INFO("server.loading", ">> mod-quest-minimap-points: {} of {} quests have a minimap point", pointed, total);
    }
};
}

void Addmod_quest_minimap_pointsScripts()
{
    new QuestMinimapPointsWorld();
}
