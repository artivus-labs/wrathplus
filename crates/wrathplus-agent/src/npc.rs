//! What an NPC offers, read from its NPC flags before it is spoken to: the names `AgentNearby`'s
//! `npc` list and `AgentFindNpc` use.

use wrathplus_protocol::types::NpcService;

/// The NPC services an agent can ask for, by name: `NpcService`'s flags, with trainers also split
/// by kind (AzerothCore `UNIT_NPC_FLAG_TRAINER_CLASS` 0x20, `_PROFESSION` 0x40).
pub(crate) const SERVICES: &[(&str, Option<NpcService>, u32)] = &[
    ("gossip", Some(NpcService::Gossip), 0),
    ("questgiver", Some(NpcService::QuestGiver), 0),
    ("vendor", Some(NpcService::Vendor), 0),
    ("repair", Some(NpcService::Repair), 0),
    ("trainer", Some(NpcService::Trainer), 0),
    ("classtrainer", None, 0x20),
    ("professiontrainer", None, 0x40),
    ("flightmaster", Some(NpcService::FlightMaster), 0),
    ("innkeeper", Some(NpcService::Innkeeper), 0),
    ("banker", Some(NpcService::Banker), 0),
    ("auctioneer", Some(NpcService::Auctioneer), 0),
    ("stablemaster", Some(NpcService::StableMaster), 0),
    ("spirithealer", Some(NpcService::SpiritHealer), 0),
    ("spiritguide", Some(NpcService::SpiritGuide), 0),
    ("battlemaster", Some(NpcService::BattleMaster), 0),
    ("tabarddesigner", Some(NpcService::TabardDesigner), 0),
    ("petitioner", Some(NpcService::Petitioner), 0),
    ("guildbanker", Some(NpcService::GuildBanker), 0),
    ("mailbox", Some(NpcService::Mailbox), 0),
];

/// The service names an NPC flags word offers.
pub(crate) fn services(bits: u32) -> Vec<&'static str> {
    let flags = wrathplus_protocol::types::NpcFlags::from_bits(bits);
    SERVICES
        .iter()
        .filter(|(_, service, mask)| match service {
            Some(service) => flags.has(*service),
            None => bits & mask != 0,
        })
        .map(|(name, _, _)| *name)
        .collect()
}
