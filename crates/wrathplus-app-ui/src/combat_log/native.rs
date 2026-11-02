//! Build-12340 combat-log entries for `COMBAT_LOG_EVENT(_UNFILTERED)` (`https://github.com/artivus-labs/wrathplus/wiki/WRATH-COMBAT-LOG`).
//! The combat-log handlers ([`super`]) record each packet here as it lands; [`feed`] stamps the
//! unit flags (`0x74dcb0`, [`super::native_flags`]) and names, then hands each entry to Native UI's
//! store (`push_wrath_combat_log`), in wire order.
//!
//! Event indices and suffix groups follow `0x74e290` and the event table `0xadb758`; the builders
//! are `0x751920`..`0x753100` (`/tmp/azb-app-mig/ghidra/combatlog2-*.log`). Every spell entry
//! needs a named, logged spell (`Attributes & 0x180` clear); an aura entry a non-passive one.
//!
//! Inferred and labelled: the swing miss type by victim state; the spell school as `1 << index`
//! (the loader keeps a 12340 mask's lowest school only); names off the name caches, nil while
//! unknown. Aura entries diff the packet snapshots (`0x72f5d0` not traced in full): a unit's first
//! snapshot logs nothing, a re-sent slot with no change reads as a refresh, and the
//! `AttributesEx6 & 0x400` gate is not applied. `SPELL_CAST_START`/`SUCCESS` skip the unit check
//! `+0x148`; `SPELL_CAST_FAILED` is our own result with its unfilled `SPELL_FAILED_*` text. The
//! execute log maps by AzerothCore effect (its dispatcher is not a defined function); a durability
//! or enchant entry's item name is nil. `UNIT_DIED` (or a totem's `UNIT_DESTROYED`, a gas cloud's
//! `UNIT_DISSIPATES`) is a unit's health reaching 0. A spell-log miss and an immune target are the
//! damage-shield misses `0x751b80` builds, miss type only. Not built: the aura-broken pair (opcode
//! `0x14F`, which AzerothCore never sends), building damage, the cast's own miss list.
use std::collections::{BTreeSet, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

use wrathplus_protocol::types::{
    DamageShield, DispelFailed, EnchantmentLog, ExecuteLog, PartyKillLog, PeriodicAuraLog,
    PeriodicTick, SpellDamageLog, SpellDispelLog, SpellEnergizeLog, SpellHealLog,
    SpellInstaKillLog, SpellLogExecute, SpellLogMiss, SpellOutcomeLog,
};
use wrathplus_protocol::wrath_auras::NativeAura;
use wrathplus_protocol::SessionEvent;
use wrathplus_ui::script::ScriptValue as V;

/// Indices into the native event table (`0xadb758`).
pub(crate) mod event {
    pub(crate) const ENVIRONMENTAL_DAMAGE: u8 = 0;
    pub(crate) const SWING_DAMAGE: u8 = 1;
    pub(crate) const SWING_MISSED: u8 = 2;
    pub(crate) const SPELL_CAST_START: u8 = 5;
    pub(crate) const SPELL_CAST_SUCCESS: u8 = 6;
    pub(crate) const SPELL_CAST_FAILED: u8 = 7;
    pub(crate) const SPELL_MISSED: u8 = 8;
    pub(crate) const SPELL_DAMAGE: u8 = 9;
    pub(crate) const SPELL_HEAL: u8 = 10;
    pub(crate) const SPELL_ENERGIZE: u8 = 11;
    pub(crate) const SPELL_DRAIN: u8 = 12;
    pub(crate) const SPELL_INSTAKILL: u8 = 14;
    pub(crate) const SPELL_SUMMON: u8 = 15;
    pub(crate) const SPELL_CREATE: u8 = 16;
    pub(crate) const SPELL_INTERRUPT: u8 = 17;
    pub(crate) const SPELL_EXTRA_ATTACKS: u8 = 18;
    pub(crate) const SPELL_DURABILITY_DAMAGE: u8 = 19;
    pub(crate) const SPELL_DURABILITY_DAMAGE_ALL: u8 = 20;
    pub(crate) const SPELL_AURA_APPLIED: u8 = 21;
    pub(crate) const SPELL_AURA_APPLIED_DOSE: u8 = 22;
    pub(crate) const SPELL_AURA_REMOVED_DOSE: u8 = 23;
    pub(crate) const SPELL_AURA_REMOVED: u8 = 24;
    pub(crate) const SPELL_AURA_REFRESH: u8 = 25;
    pub(crate) const SPELL_DISPEL: u8 = 26;
    pub(crate) const SPELL_STOLEN: u8 = 27;
    pub(crate) const ENCHANT_APPLIED: u8 = 31;
    pub(crate) const ENCHANT_REMOVED: u8 = 32;
    pub(crate) const SPELL_PERIODIC_MISSED: u8 = 33;
    pub(crate) const SPELL_PERIODIC_DAMAGE: u8 = 34;
    pub(crate) const SPELL_PERIODIC_HEAL: u8 = 35;
    pub(crate) const SPELL_PERIODIC_ENERGIZE: u8 = 36;
    pub(crate) const SPELL_PERIODIC_LEECH: u8 = 38;
    pub(crate) const SPELL_DISPEL_FAILED: u8 = 39;
    pub(crate) const DAMAGE_SHIELD: u8 = 40;
    pub(crate) const DAMAGE_SHIELD_MISSED: u8 = 41;
    pub(crate) const DAMAGE_SPLIT: u8 = 42;
    pub(crate) const PARTY_KILL: u8 = 43;
    pub(crate) const UNIT_DIED: u8 = 44;
    pub(crate) const UNIT_DESTROYED: u8 = 45;
    pub(crate) const SPELL_RESURRECT: u8 = 46;
    pub(crate) const UNIT_DISSIPATES: u8 = 49;
}

/// What the feed fills in before an entry goes out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Fill {
    #[default]
    None,
    /// `suffix[0]` is an extra spell id; group `0x100` (id, name, school) replaces it.
    ExtraSpell,
    /// `suffix[0]` is a `SPELL_FAILED_*` key; group `0x2`'s string is its GlobalStrings text.
    FailedKey,
    /// `suffix[0]` is an item entry; group `0x200` is the id and a nil name (labelled).
    Item,
    /// `suffix[0]` is a `SpellItemEnchantment` id, which group `0x1000` replaces with its name
    /// (`0x751050`); `suffix[1]` is the item entry, as [`Fill::Item`]. The entry drops for an
    /// enchant with no row or with `Flags & 0x2`.
    Enchant,
}

/// The aura builders' own gates, checked against the spell record in the feed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuraGate {
    /// `0x752710`/`0x752ba0`: not a passive spell.
    Shown,
    /// `0x752710`'s removal: also only while the unit lives.
    Removed,
    /// `0x752860`: the spell stacks (`StackAmount > 1`).
    Dose,
}

/// One entry before its units are flagged and named.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pending {
    pub(crate) timestamp: f64,
    pub(crate) event: u8,
    pub(crate) source: u64,
    pub(crate) dest: u64,
    pub(crate) spell: Option<u32>,
    pub(crate) suffix: Vec<V>,
    pub(crate) fill: Fill,
    pub(crate) aura: Option<AuraGate>,
}

/// One held aura slot, for the snapshot diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Held {
    spell: u32,
    stacks: u8,
    caster: u64,
    debuff: bool,
}

/// Each unit's last aura snapshot by slot.
#[derive(Default)]
pub(crate) struct AuraSnapshots(HashMap<u64, HashMap<u8, Held>>);

/// The units whose aura snapshots are kept; past it the memo starts over.
const AURA_UNITS: usize = 4096;
/// Entries held while no feed drains them (the in-game UI down); the oldest half drops past it.
const PENDING_BOUND: usize = 8192;

/// The entries recorded this frame, in wire order, and the aura snapshots they diff.
#[derive(bevy::prelude::Resource, Default)]
pub(crate) struct NativeCombatLog(pub(crate) Vec<Pending>, pub(crate) AuraSnapshots);

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

/// `0x74e290`'s optional number: nil for 0.
fn opt(n: u32) -> V {
    if n == 0 {
        V::Nil
    } else {
        V::Int(i64::from(n))
    }
}
fn flag(on: bool) -> V {
    if on {
        V::Int(1)
    } else {
        V::Nil
    }
}
fn int(n: impl Into<i64>) -> V {
    V::Int(n.into())
}
/// Group `0x400`'s aura type (`0x74e290`: a zero byte is `BUFF`).
fn aura_type(debuff: bool) -> V {
    V::Str(if debuff { "DEBUFF" } else { "BUFF" }.into())
}
/// A game object's guid, which `0x7522a0` logs as created rather than summoned.
fn is_gameobject(guid: u64) -> bool {
    (guid >> 32) as u32 & 0xf0f0_0000 == 0xf010_0000
}

/// Group `0x10`: amount, overkill, school, resisted, blocked, absorbed, critical, glancing,
/// crushing.
#[allow(clippy::too_many_arguments)]
fn damage(
    amount: u32,
    overkill: u32,
    school_mask: u32,
    resisted: u32,
    blocked: u32,
    absorbed: u32,
    critical: bool,
    glancing: bool,
    crushing: bool,
) -> Vec<V> {
    vec![
        int(amount),
        int(overkill),
        int(school_mask),
        opt(resisted),
        opt(blocked),
        opt(absorbed),
        flag(critical),
        flag(glancing),
        flag(crushing),
    ]
}

/// Group `0x20`: amount, overhealing, absorbed, critical.
fn heal(amount: u32, overheal: u32, absorbed: u32, critical: bool) -> Vec<V> {
    vec![int(amount), int(overheal), int(absorbed), flag(critical)]
}

/// AzerothCore `SpellMissInfo`, which is the miss-type table `0xadbfac` index for index.
const MISS_TYPES: [&str; 12] = [
    "NONE", "MISS", "RESIST", "DODGE", "PARRY", "BLOCK", "EVADE", "IMMUNE", "IMMUNE", "DEFLECT",
    "ABSORB", "REFLECT",
];
/// `SPELL_MISS_IMMUNE`, the type `0x6ce3d0` passes.
const MISS_IMMUNE: u8 = 7;
/// The environmental types (`0xad671c`), by AzerothCore `EnviromentalDamage`.
const ENVIRONMENTAL: [&str; 6] = ["FATIGUE", "DROWNING", "FALLING", "LAVA", "SLIME", "FIRE"];

/// AzerothCore's spell hit-info critical bit (`SPELL_HIT_TYPE_CRIT`).
const SPELL_HIT_CRIT: u32 = 0x2;
/// AzerothCore's `SPELL_HIT_TYPE_SPLIT`, the bit the `0x6d3840` handler sends to `0x752de0`.
const SPELL_HIT_SPLIT: u32 = 0x8;

impl NativeCombatLog {
    fn push(&mut self, event: u8, source: u64, dest: u64, spell: Option<u32>, suffix: Vec<V>) {
        self.push_with(event, source, dest, spell, suffix, Fill::None, None);
    }

    #[allow(clippy::too_many_arguments)]
    fn push_with(
        &mut self,
        event: u8,
        source: u64,
        dest: u64,
        spell: Option<u32>,
        suffix: Vec<V>,
        fill: Fill,
        aura: Option<AuraGate>,
    ) {
        if self.0.len() >= PENDING_BOUND {
            self.0.drain(..PENDING_BOUND / 2);
        }
        self.0.push(Pending {
            timestamp: now(),
            event,
            source,
            dest,
            spell,
            suffix,
            fill,
            aura,
        });
    }

    /// Record the native entries one shared combat event carries; other events record nothing.
    pub(crate) fn record(&mut self, ev: &SessionEvent) {
        use event::*;
        match ev {
            // `0x751920`, `0x7519e0`: a start logs no destination, a launch its unit target.
            SessionEvent::SpellStart {
                caster, spell_id, ..
            } => self.push(SPELL_CAST_START, *caster, 0, Some(*spell_id), vec![]),
            SessionEvent::SpellGo {
                caster,
                spell_id,
                target,
                ..
            } => self.push(
                SPELL_CAST_SUCCESS,
                *caster,
                target.unwrap_or(0),
                Some(*spell_id),
                vec![],
            ),
            // `0x751ad0`: our own refused cast, with the reason's text.
            SessionEvent::WrathSpellFailure(f) => {
                if let Some(key) = crate::spell::cast_fail_keys::wrath_key(f.reason) {
                    self.push_with(
                        SPELL_CAST_FAILED,
                        f.caster,
                        0,
                        Some(f.spell_id),
                        vec![V::Str(key.into())],
                        Fill::FailedKey,
                        None,
                    );
                }
            }
            SessionEvent::UnitAuras {
                guid,
                auras,
                updated_slots,
            } => self.auras(*guid, auras, updated_slots),
            SessionEvent::ObjectDestroyed(guid) => {
                self.1 .0.remove(guid);
            }
            SessionEvent::ObjectsRemoved(guids) => {
                for guid in guids {
                    self.1 .0.remove(guid);
                }
            }
            SessionEvent::SpellLogExecute(x) => self.execute(x),
            SessionEvent::WrathSpellLog {
                packet: wrathplus_protocol::wrath_logs::Packet::SpellSteal(log),
            } => self.dispel(SPELL_STOLEN, log),
            SessionEvent::WrathAttackerState(s) => self.swing(s),
            SessionEvent::SpellDamageLog(SpellDamageLog {
                target,
                attacker,
                spell_id,
                damage: amount,
                school,
                absorb,
                resist,
                periodic,
                blocked,
                hit_info,
                overkill,
            }) => self.push(
                // `0x6d3840`: a split share is `DAMAGE_SPLIT` (`0x752de0`, the same damage group)
                // ahead of the periodic test.
                if hit_info & SPELL_HIT_SPLIT != 0 {
                    DAMAGE_SPLIT
                } else if *periodic {
                    SPELL_PERIODIC_DAMAGE
                } else {
                    SPELL_DAMAGE
                },
                *attacker,
                *target,
                Some(*spell_id),
                damage(
                    *amount,
                    *overkill,
                    1 << u32::from(*school).min(31),
                    (*resist).max(0) as u32,
                    *blocked,
                    *absorb,
                    hit_info & SPELL_HIT_CRIT != 0,
                    false,
                    false,
                ),
            ),
            SessionEvent::PeriodicAuraLog(PeriodicAuraLog {
                target,
                caster,
                spell_id,
                ticks,
            }) => {
                for tick in ticks {
                    let (event, suffix) = match *tick {
                        PeriodicTick::Damage {
                            amount,
                            school,
                            absorb,
                            resist,
                            overkill,
                            critical,
                        } => (
                            SPELL_PERIODIC_DAMAGE,
                            damage(
                                amount,
                                overkill,
                                1 << school.min(31),
                                resist.max(0) as u32,
                                0,
                                absorb,
                                critical,
                                false,
                                false,
                            ),
                        ),
                        PeriodicTick::Heal {
                            amount,
                            overheal,
                            absorb,
                            critical,
                        } => (
                            SPELL_PERIODIC_HEAL,
                            heal(amount, overheal, absorb, critical),
                        ),
                        PeriodicTick::Energize { power, amount } => {
                            (SPELL_PERIODIC_ENERGIZE, vec![int(amount), int(power)])
                        }
                        // Group `0x80`: amount, power type, extra amount (nil for 0).
                        PeriodicTick::ManaLeech {
                            power,
                            amount,
                            multiplier,
                        } => (
                            SPELL_PERIODIC_LEECH,
                            vec![
                                int(amount),
                                int(power),
                                opt((amount as f32 * multiplier) as u32),
                            ],
                        ),
                    };
                    self.push(event, *caster, *target, Some(*spell_id), suffix);
                }
            }
            SessionEvent::SpellHealLog(SpellHealLog {
                target,
                healer,
                spell_id,
                amount,
                critical,
                overheal,
                absorb,
            }) => self.push(
                SPELL_HEAL,
                *healer,
                *target,
                Some(*spell_id),
                heal(*amount, *overheal, *absorb, *critical),
            ),
            SessionEvent::SpellEnergizeLog(SpellEnergizeLog {
                target,
                caster,
                spell_id,
                power,
                amount,
            }) => self.push(
                SPELL_ENERGIZE,
                *caster,
                *target,
                Some(*spell_id),
                vec![int(*amount), int(*power)],
            ),
            SessionEvent::DamageShield(DamageShield {
                victim,
                attacker,
                damage: amount,
                school,
                spell_id,
                overkill,
            }) => self.push(
                DAMAGE_SHIELD,
                *attacker,
                *victim,
                Some(*spell_id),
                damage(
                    *amount,
                    *overkill,
                    1 << (*school).min(31),
                    0,
                    0,
                    0,
                    false,
                    false,
                    false,
                ),
            ),
            SessionEvent::SpellLogMiss(SpellLogMiss {
                spell_id,
                caster,
                misses,
            }) => {
                // `0x6d3090` passes `0x751b80` its damage-shield flag as a constant 1: each row
                // is `DAMAGE_SHIELD_MISSED`, and a row with no miss type logs nothing.
                for &(target, code) in misses {
                    self.shield_miss(DAMAGE_SHIELD_MISSED, *caster, target, *spell_id, code);
                }
            }
            // `0x6ce3d0` (registered for `0x263` by `0x6e83b0`): `0x751b80` with `IMMUNE`, its
            // damage-shield flag set by a zero log-format byte and its periodic flag otherwise.
            SessionEvent::SpellOrDamageImmune(SpellOutcomeLog {
                caster,
                target,
                spell_id,
                log_format,
            }) => self.shield_miss(
                if *log_format == 0 {
                    DAMAGE_SHIELD_MISSED
                } else {
                    SPELL_PERIODIC_MISSED
                },
                *caster,
                *target,
                *spell_id,
                MISS_IMMUNE,
            ),
            // `0x750d40`: a resisted proc is `SPELL_MISSED` `RESIST`, group `0x4000`'s integer 0.
            SessionEvent::ProcResist(SpellOutcomeLog {
                caster,
                target,
                spell_id,
                ..
            }) => self.push(
                SPELL_MISSED,
                *caster,
                *target,
                Some(*spell_id),
                vec![V::Str("RESIST".into()), int(0)],
            ),
            // `0x751050`: no spell, and `ENCHANT_REMOVED` for an empty caster. Source and
            // destination by AzerothCore's field names: the caster, the item's owner.
            SessionEvent::EnchantmentLog(EnchantmentLog {
                target,
                caster,
                item_entry,
                enchant_id,
            }) => self.push_with(
                if *caster == 0 {
                    ENCHANT_REMOVED
                } else {
                    ENCHANT_APPLIED
                },
                *caster,
                *target,
                None,
                vec![int(*enchant_id), int(*item_entry)],
                Fill::Enchant,
                None,
            ),
            SessionEvent::SpellInstaKillLog(SpellInstaKillLog {
                victim,
                spell_id,
                caster,
            }) => self.push(SPELL_INSTAKILL, *caster, *victim, Some(*spell_id), vec![]),
            SessionEvent::SpellDispelLog(log) => self.dispel(SPELL_DISPEL, log),
            SessionEvent::DispelFailed(DispelFailed {
                caster,
                victim,
                spell_id,
                spell_ids,
            }) => {
                for failed in spell_ids {
                    self.push_with(
                        SPELL_DISPEL_FAILED,
                        *caster,
                        *victim,
                        Some(*spell_id),
                        vec![int(*failed)],
                        Fill::ExtraSpell,
                        None,
                    );
                }
            }
            SessionEvent::PartyKillLog(PartyKillLog { killer, victim }) => {
                self.push(PARTY_KILL, *killer, *victim, None, vec![])
            }
            SessionEvent::WrathEnvironmentalDamage(e) => {
                let kind = ENVIRONMENTAL
                    .get(usize::from(e.damage_type))
                    .copied()
                    .unwrap_or("FATIGUE");
                let mut suffix = vec![V::Str(kind.into())];
                // Environmental damage has no school word on the wire: physical.
                suffix.extend(damage(
                    e.damage, 0, 1, e.resisted, 0, e.absorbed, false, false, false,
                ));
                self.push(ENVIRONMENTAL_DAMAGE, 0, e.victim, None, suffix);
            }
            _ => {}
        }
    }

    /// `0x751b80`'s miss through the template `0x750500`: the miss type only (labelled: the
    /// template's `0x74d980` amount groups are not traced), and nothing for a zero type.
    fn shield_miss(&mut self, event: u8, source: u64, dest: u64, spell: u32, code: u8) {
        if code == 0 {
            return;
        }
        let miss = MISS_TYPES.get(usize::from(code)).copied().unwrap_or("MISS");
        self.push(event, source, dest, Some(spell), vec![V::Str(miss.into())]);
    }

    /// `0x752930`: each removed aura as the extra spell, its byte the aura type (a zero `BUFF`).
    fn dispel(&mut self, event: u8, log: &SpellDispelLog) {
        for aura in &log.auras {
            self.push_with(
                event,
                log.caster,
                log.victim,
                Some(log.spell_id),
                vec![int(aura.spell_id), aura_type(aura.flag != 0)],
                Fill::ExtraSpell,
                None,
            );
        }
    }

    /// The execute log's rows by effect (AzerothCore `SpellEffects`, labelled): drain `0x751c40`;
    /// extra attacks `0x752520`; interrupt `0x7523d0`; durability `0x7525e0`/`0x752680`;
    /// resurrect `0x752340`; a summon or created object `0x7522a0`, split by the guid's kind.
    fn execute(&mut self, x: &SpellLogExecute) {
        use event::*;
        let spell = Some(x.spell_id);
        for (effect, rows) in &x.effects {
            for row in rows {
                match (*effect, row) {
                    (
                        8,
                        ExecuteLog::PowerDrain {
                            target,
                            amount,
                            power,
                            multiplier,
                        },
                    ) => self.push(
                        SPELL_DRAIN,
                        x.caster,
                        *target,
                        spell,
                        vec![
                            int(*amount),
                            int(*power),
                            opt((*amount as f32 * multiplier) as u32),
                        ],
                    ),
                    (_, ExecuteLog::ExtraAttacks { target, count }) => self.push(
                        SPELL_EXTRA_ATTACKS,
                        x.caster,
                        *target,
                        spell,
                        vec![int(*count)],
                    ),
                    (_, ExecuteLog::InterruptCast { target, spell_id }) => self.push_with(
                        SPELL_INTERRUPT,
                        x.caster,
                        *target,
                        spell,
                        vec![int(*spell_id)],
                        Fill::ExtraSpell,
                        None,
                    ),
                    (
                        _,
                        ExecuteLog::DurabilityDamage {
                            target, item_entry, ..
                        },
                    ) => {
                        if *item_entry < 0 {
                            self.push(
                                SPELL_DURABILITY_DAMAGE_ALL,
                                x.caster,
                                *target,
                                spell,
                                vec![],
                            )
                        } else {
                            self.push_with(
                                SPELL_DURABILITY_DAMAGE,
                                x.caster,
                                *target,
                                spell,
                                vec![int(*item_entry)],
                                Fill::Item,
                                None,
                            )
                        }
                    }
                    (18 | 113, ExecuteLog::Target { target }) => {
                        self.push(SPELL_RESURRECT, x.caster, *target, spell, vec![])
                    }
                    (28 | 50 | 56 | 76 | 83 | 104..=107, ExecuteLog::Target { target }) => {
                        let event = if is_gameobject(*target) {
                            SPELL_CREATE
                        } else {
                            SPELL_SUMMON
                        };
                        self.push(event, x.caster, *target, spell, vec![])
                    }
                    _ => {}
                }
            }
        }
    }

    /// Diff a unit's aura snapshot against the last (`0x752710`, `0x752860`, `0x752ba0`): a new
    /// spell in a slot is applied, a vacated slot removed, a stack change a dose, an unchanged
    /// re-sent slot a refresh. A unit's first snapshot logs nothing.
    fn auras(&mut self, guid: u64, auras: &[NativeAura], updated: &[u8]) {
        use event::*;
        let next: HashMap<u8, Held> = auras
            .iter()
            .map(|a| {
                (
                    a.slot,
                    Held {
                        spell: a.spell_id,
                        stacks: a.stacks,
                        caster: a.caster_guid.unwrap_or(guid),
                        debuff: !a.is_helpful(),
                    },
                )
            })
            .collect();
        let memo = &mut self.1 .0;
        if memo.len() >= AURA_UNITS && !memo.contains_key(&guid) {
            memo.clear();
        }
        let Some(prev) = memo.insert(guid, next.clone()) else {
            return;
        };
        let slots: BTreeSet<u8> = updated
            .iter()
            .copied()
            .chain(prev.keys().copied().filter(|s| !next.contains_key(s)))
            .collect();
        for slot in slots {
            let (was, now) = (prev.get(&slot).copied(), next.get(&slot).copied());
            let gone = |log: &mut Self, h: Held| {
                log.push_with(
                    SPELL_AURA_REMOVED,
                    h.caster,
                    guid,
                    Some(h.spell),
                    vec![aura_type(h.debuff)],
                    Fill::None,
                    Some(AuraGate::Removed),
                )
            };
            let came = |log: &mut Self, h: Held| {
                log.push_with(
                    SPELL_AURA_APPLIED,
                    h.caster,
                    guid,
                    Some(h.spell),
                    vec![aura_type(h.debuff)],
                    Fill::None,
                    Some(AuraGate::Shown),
                )
            };
            match (was, now) {
                (Some(p), None) => gone(self, p),
                (None, Some(n)) => came(self, n),
                (Some(p), Some(n)) if p.spell != n.spell => {
                    gone(self, p);
                    came(self, n);
                }
                (Some(p), Some(n)) if p.stacks != n.stacks => self.push_with(
                    if n.stacks > p.stacks {
                        SPELL_AURA_APPLIED_DOSE
                    } else {
                        SPELL_AURA_REMOVED_DOSE
                    },
                    n.caster,
                    guid,
                    Some(n.spell),
                    vec![aura_type(n.debuff), int(n.stacks)],
                    Fill::None,
                    Some(AuraGate::Dose),
                ),
                (Some(_), Some(n)) => self.push_with(
                    SPELL_AURA_REFRESH,
                    n.caster,
                    guid,
                    Some(n.spell),
                    vec![aura_type(n.debuff)],
                    Fill::None,
                    Some(AuraGate::Shown),
                ),
                (None, None) => {}
            }
        }
    }

    /// `0x7512b0`: a landed swing is `SWING_DAMAGE` with the two school rows summed; a miss is
    /// `SWING_MISSED` with its type (labelled: the type is read off AC's victim state and the
    /// full-absorb/full-resist hit bits, the swing's `0x4004` arm not fully traced).
    fn swing(&mut self, s: &wrathplus_protocol::wrath_combat::AttackerState) {
        const HIT_CRIT: u32 = 0x200;
        const HIT_GLANCING: u32 = 0x1_0000;
        const HIT_CRUSHING: u32 = 0x2_0000;
        const HIT_FULL_ABSORB: u32 = 0x20 | 0x40;
        const HIT_FULL_RESIST: u32 = 0x80 | 0x100;
        let (absorbed, resisted) = s.damages.iter().fold((0, 0), |(a, r), d| {
            (a + d.absorb.unwrap_or(0), r + d.resist.unwrap_or(0))
        });
        let school = s.damages.first().map_or(1, |d| d.school_mask);
        let blocked = s.blocked.unwrap_or(0);
        if s.damage > 0 {
            self.push(
                event::SWING_DAMAGE,
                s.attacker,
                s.victim,
                None,
                damage(
                    s.damage,
                    s.overkill,
                    school,
                    resisted,
                    blocked,
                    absorbed,
                    s.hit_info & HIT_CRIT != 0,
                    s.hit_info & HIT_GLANCING != 0,
                    s.hit_info & HIT_CRUSHING != 0,
                ),
            );
            return;
        }
        // AzerothCore `VictimState`: 0 intact (a miss), 2 dodge, 3 parry, 5 blocks, 6 evades,
        // 7 immune, 8 deflects.
        let (miss, amount) = if s.hit_info & HIT_FULL_ABSORB != 0 {
            ("ABSORB", absorbed)
        } else if s.hit_info & HIT_FULL_RESIST != 0 {
            ("RESIST", resisted)
        } else {
            match s.target_state {
                2 => ("DODGE", 0),
                3 => ("PARRY", 0),
                5 => ("BLOCK", blocked),
                6 => ("EVADE", 0),
                7 => ("IMMUNE", 0),
                8 => ("DEFLECT", 0),
                _ => ("MISS", 0),
            }
        };
        let mut suffix = vec![V::Str(miss.into())];
        if amount != 0 {
            suffix.push(int(amount));
        }
        self.push(event::SWING_MISSED, s.attacker, s.victim, None, suffix);
    }
}

/// The feed's inputs beside the VM and the queue.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct FeedInputs<'w, 's> {
    names: bevy::prelude::Res<'w, crate::names::NameCache>,
    commands: bevy::prelude::Res<'w, crate::net::NetCommands>,
    self_guid: bevy::prelude::Res<'w, crate::app_core::session::SelfGuid>,
    selection: bevy::prelude::Res<'w, crate::app_core::state::Selection>,
    group: Option<bevy::prelude::Res<'w, crate::app_core::group::GroupState>>,
    index: bevy::prelude::Res<'w, crate::net::GuidIndex>,
    stores: bevy::prelude::Query<'w, 's, &'static crate::app_core::session::ObjectStore>,
    /// The `HOSTILE`/`FRIENDLY` flags' reaction, with the group roster, forced reactions and
    /// owner lookup.
    relations: crate::target::RelationInputs<'w, 's>,
    spells: Option<bevy::prelude::Res<'w, crate::app_core::state::Spells>>,
    /// The enchant names and gate of `ENCHANT_APPLIED`/`REMOVED`.
    enchants: Option<bevy::prelude::Res<'w, crate::items::Enchants>>,
    /// The totem slots `UNIT_DESTROYED` reads.
    summon_slots: Option<bevy::prelude::Res<'w, crate::ui_companions::SummonSlots>>,
    /// The units whose descriptors changed this frame, for `UNIT_DIED`.
    changed: bevy::prelude::Query<
        'w,
        's,
        (
            &'static crate::net::Guid,
            &'static crate::app_core::session::ObjectStore,
        ),
        bevy::prelude::Changed<crate::app_core::session::ObjectStore>,
    >,
    /// Each unit's last health; past [`AURA_UNITS`] it starts over.
    health: bevy::prelude::Local<'s, HashMap<u64, u32>>,
}

/// `CreatureType.dbc` 13, Gas Cloud.
const CREATURE_TYPE_GAS_CLOUD: u32 = 13;
/// `SPELL_EFFECT_SUMMON`.
const SPELL_EFFECT_SUMMON: u32 = 28;

/// `0x752ed0`'s event for a dying unit: `UNIT_DISSIPATES` for a gas cloud (its creature template's
/// type), else `UNIT_DESTROYED` for a unit in a totem slot (its `UNIT_CREATED_BY_SPELL`'s first
/// effect is SUMMON, with a `SummonProperties` Slot of 1-4), else `UNIT_DIED`.
pub(crate) fn death_event(creature_type: Option<u32>, summon_slot: Option<u32>) -> u8 {
    if creature_type == Some(CREATURE_TYPE_GAS_CLOUD) {
        event::UNIT_DISSIPATES
    } else if summon_slot.is_some_and(|slot| (1..=4).contains(&slot)) {
        event::UNIT_DESTROYED
    } else {
        event::UNIT_DIED
    }
}

/// `0x752ed0`: each unit whose health fell to 0 since its last reading dies, in no particular
/// order among this frame's deaths, with the event `kind` picks ([`death_event`]).
pub(crate) fn deaths<'a>(
    changed: impl IntoIterator<Item = (u64, &'a crate::app_core::session::ObjectStore)>,
    health: &mut HashMap<u64, u32>,
    log: &mut NativeCombatLog,
    kind: impl Fn(&crate::app_core::session::ObjectStore) -> u8,
) {
    for (guid, store) in changed {
        if !store.is_unit() {
            continue;
        }
        let Some(now) = store.0.unit_health() else {
            continue;
        };
        if health.len() >= AURA_UNITS && !health.contains_key(&guid) {
            health.clear();
        }
        if health.insert(guid, now).is_some_and(|was| was > 0) && now == 0 {
            log.push(kind(store), 0, guid, None, vec![]);
        }
    }
}

/// `0x751050`'s gate and group `0x1000` string: `None` drops an enchant with no row or with
/// `Flags & 0x2`; a kept one is its name, nil when empty. With no table loaded every entry passes,
/// unnamed.
pub(crate) fn enchant_name(enchants: Option<&crate::items::Enchants>, id: u32) -> Option<V> {
    let Some(enchants) = enchants else {
        return Some(V::Nil);
    };
    (enchants.0.has_row(id) && !enchants.0.tooltip_hides_name(id)).then(|| {
        enchants
            .0
            .name(id)
            .map_or(V::Nil, |name| V::Str(name.into()))
    })
}

/// Flag, name and hand each recorded entry to Native UI's store, in wire order. A Vanilla VM
/// drops them. Labelled: no focus unit is tracked here, so `FOCUS` never sets.
pub(super) fn feed(
    script: Option<bevy::prelude::NonSendMut<wrathplus_ui::script::UiScript>>,
    mut log: bevy::prelude::ResMut<NativeCombatLog>,
    inputs: FeedInputs,
) {
    use wrathplus_ui::script::{WrathCombatLogEntry, WrathCombatLogUnit};
    let FeedInputs {
        names,
        commands,
        self_guid,
        selection,
        group,
        index,
        stores,
        relations,
        spells,
        enchants,
        summon_slots,
        changed,
        mut health,
    } = inputs;
    let catalog = spells.as_deref().map(|s| &s.catalog);
    deaths(
        changed.iter().map(|(guid, store)| (guid.0, store)),
        &mut health,
        &mut log,
        |store| {
            let fields = &store.0;
            let creature_type = fields
                .object_entry()
                .and_then(|entry| names.creature_type(entry));
            let summon_slot = fields
                .unit_created_by_spell()
                .and_then(|id| catalog?.get(id))
                .filter(|d| d.effects[0] == SPELL_EFFECT_SUMMON)
                .and_then(|d| u32::try_from(d.effect_misc_value_b[0]).ok())
                .and_then(|row| summon_slots.as_deref()?.0.slot(row));
            death_event(creature_type, summon_slot)
        },
    );
    let entries = std::mem::take(&mut log.0);
    let Some(mut script) = script else {
        return;
    };
    if entries.is_empty() {
        return;
    }
    let me = self_guid.0.unwrap_or(0);
    let store = |guid: u64| crate::target::guid_store(Some(&index), &stores, guid);
    let me_store = store(me);
    let reaction = |unit: &crate::app_core::session::ObjectStore| {
        relations.ring_reaction(Some(unit), me_store)
    };
    let flags_in = super::native_flags::FlagInputs {
        me,
        target: selection.guid.unwrap_or(0),
        focus: 0,
        group: group.as_deref(),
        store: &store,
        reaction: &reaction,
    };
    let spell = |id: u32| -> (String, u32) {
        catalog
            .and_then(|c| c.get(id))
            .map_or((String::new(), 0), |d| {
                (d.name.clone(), 1 << d.school.min(31))
            })
    };
    // The builders' record gates: a named spell not hidden from the log (`Attributes & 0x180`),
    // and the aura gates. With no catalog loaded every entry passes.
    let admitted = |p: &Pending| {
        if let (Fill::Enchant, Some(&V::Int(id))) = (p.fill, p.suffix.first()) {
            return enchant_name(enchants.as_deref(), id as u32).is_some();
        }
        let (Some(catalog), Some(id)) = (catalog, p.spell) else {
            return true;
        };
        let Some(d) = catalog.get(id) else {
            return false;
        };
        if d.attributes & 0x180 != 0 || d.name.is_empty() {
            return false;
        }
        match p.aura {
            None => true,
            Some(AuraGate::Dose) => d.stack_amount > 1,
            Some(AuraGate::Shown) => d.attributes & 0x40 == 0,
            Some(AuraGate::Removed) => {
                d.attributes & 0x40 == 0
                    && store(p.dest)
                        .and_then(|s| s.0.unit_health())
                        .is_none_or(|h| h > 0)
            }
        }
    };
    let unit = |guid: u64| {
        (guid != 0).then(|| WrathCombatLogUnit {
            guid,
            name: names
                .resolve_unit(guid, store(guid), &commands)
                .map(str::to_string),
            flags: super::native_flags::unit_flags(guid, &flags_in),
        })
    };
    for mut p in entries {
        if !admitted(&p) {
            continue;
        }
        match (p.fill, p.suffix.first().cloned()) {
            (Fill::ExtraSpell, Some(V::Int(id))) => {
                let (name, school) = spell(id as u32);
                p.suffix
                    .splice(0..1, [V::Int(id), V::Str(name), V::Int(i64::from(school))]);
            }
            (Fill::FailedKey, Some(V::Str(key))) => {
                let text = script
                    .lua()
                    .globals()
                    .get::<String>(key.as_str())
                    .ok()
                    .filter(|t| !t.is_empty())
                    .unwrap_or(key);
                p.suffix[0] = V::Str(text);
            }
            (Fill::Item, Some(V::Int(_))) => p.suffix.push(V::Nil),
            (Fill::Enchant, Some(V::Int(id))) => {
                p.suffix[0] = enchant_name(enchants.as_deref(), id as u32).unwrap_or(V::Nil);
                p.suffix.push(V::Nil);
            }
            _ => {}
        }
        let entry = WrathCombatLogEntry {
            timestamp: p.timestamp,
            event: p.event,
            source: unit(p.source),
            dest: unit(p.dest),
            spell: p.spell.map(|id| {
                let (name, school) = spell(id);
                (id, name, school)
            }),
            suffix: p.suffix,
        };
        script.push_wrath_combat_log(entry);
    }
}

#[cfg(test)]
#[path = "native_tests.rs"]
mod tests;
