//! Wrath character-sheet values from semantic player fields, not reconstructed stats.
use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use bevy::prelude::*;
use wrathplus_assets::{AssetSet, ClientDataProfile, LockRecover, WorldAssets};
use wrathplus_formats::{GameTable, WrathGameTables};
use wrathplus_protocol::types::{Field, ObjectFields};
use wrathplus_ui::script::{UiScript, WrathPaperDollStats, WrathStatConversions};

pub struct UiPaperDollPlugin;

/// The two stat panes' selections (12340 `0x51ee5c`, `0x51ee7f`, default `""`): the stock sheet's
/// `VARIABLES_LOADED` arm fills an empty one per class (PaperDollFrame.lua:161-175), and
/// `PaperDollFrame_UpdateStats` reads both.
pub(crate) const CVARS: &[crate::cvars::Registered] = &[
    crate::cvars::same("playerStatLeftDropdown", ""),
    crate::cvars::same("playerStatRightDropdown", ""),
];

impl Plugin for UiPaperDollPlugin {
    fn build(&self, app: &mut App) {
        crate::cvars::RegisterCvars::register_cvars(app, CVARS);
        app.add_systems(Startup, load_tables.after(AssetSet::Open))
            .add_systems(Update, feed.in_set(crate::app_core::schedule::UnitFeed));
    }
}

#[derive(Resource)]
struct PaperDollTables(WrathGameTables);

fn load_tables(
    mut commands: Commands,
    assets: Option<Res<WorldAssets>>,
    profile: Option<Res<ClientDataProfile>>,
) {
    let Some(assets) = assets else { return };
    if !profile.is_some_and(|p| matches!(p.0, wrathplus_formats::DataProfile::Wrath { .. })) {
        return;
    }
    let loaded = wrathplus_formats::load_wrath_game_tables(&mut assets.chain.lock_recover());
    match loaded {
        Ok(tables) => {
            commands.insert_resource(PaperDollTables(tables));
        }
        Err(e) => warn!("paperdoll: GameTables unavailable, conversions remain zero: {e:#}"),
    }
}

// Player.cpp:5393/5416. Health is per tick (2s); mana is per second, and the
// stock PaperDollFrame.lua multiplies mana by five for its spirit tooltip.
fn spirit_regen(
    spirit: f32,
    hp_base: Option<f32>,
    hp_more: Option<f32>,
    mp: Option<f32>,
) -> (f32, f32) {
    let hp = hp_base
        .zip(hp_more)
        .map(|(base, more)| {
            let first = spirit.min(50.0);
            (first * base + (spirit - first) * more) * 2.0
        })
        .unwrap_or(0.0);
    (hp, mp.map(|ratio| spirit * ratio).unwrap_or(0.0))
}

// Player.cpp:5358. Do not use its server-side 1.0 fallback when a client row
// is unavailable; that would claim a conversion we cannot support.
fn rating_bonus(rating: u32, scalar: Option<f32>, divisor: Option<f32>) -> f32 {
    scalar
        .zip(divisor)
        .filter(|(s, d)| s.is_finite() && d.is_finite() && *d > 0.0)
        .map(|(s, d)| rating as f32 * s / d)
        .unwrap_or(0.0)
}

fn conversions(f: &ObjectFields, tables: &WrathGameTables) -> WrathStatConversions {
    let class = f.unit_class().unwrap_or(0);
    let level = f.unit_level().unwrap_or(0);
    let row = |table| tables.class_level(table, class, level);
    let stat = |index| f.unit_stat(index).unwrap_or(0) as f32;
    let (health_from_spirit, mana_from_spirit) = spirit_regen(
        stat(4),
        row(GameTable::OctRegenHp),
        row(GameTable::RegenHpPerSpirit),
        row(GameTable::RegenMpPerSpirit),
    );
    let rating_bonus = std::array::from_fn(|i| {
        rating_bonus(
            f.get_at(Field::PlayerCombatRating1, i as u16).unwrap_or(0),
            tables.class_combat_rating_scalar(class, i as u8),
            tables.combat_rating(i as u8, level),
        )
    });
    // These APIs expose only the stat contribution, not Player.cpp's base intercept.
    WrathStatConversions {
        crit_from_agility: row(GameTable::MeleeCrit)
            .map(|ratio| stat(1) * ratio * 100.0)
            .unwrap_or(0.0),
        spell_crit_from_intellect: row(GameTable::SpellCrit)
            .map(|ratio| stat(3) * ratio * 100.0)
            .unwrap_or(0.0),
        health_from_spirit,
        mana_from_spirit,
        rating_bonus,
    }
}

fn snapshot(f: &ObjectFields) -> WrathPaperDollStats {
    let float = |field| f.get_float(field, 0).unwrap_or(0.0);
    let word = |field| f.get(field).unwrap_or(0);
    WrathPaperDollStats {
        ratings: std::array::from_fn(|i| {
            f.get_at(Field::PlayerCombatRating1, i as u16).unwrap_or(0)
        }),
        dodge: float(Field::PlayerDodgePercentage),
        parry: float(Field::PlayerParryPercentage),
        block: float(Field::PlayerBlockPercentage),
        melee_crit: float(Field::PlayerCritPercentage),
        ranged_crit: float(Field::PlayerRangedCritPercentage),
        spell_crit: std::array::from_fn(|i| {
            f.get_float(Field::PlayerSpellCritPercentage1, i as u16)
                .unwrap_or(0.0)
        }),
        spell_damage: std::array::from_fn(|i| {
            // `GetSpellBonusDamage` (`0x60e310`): the signed sum, unclamped.
            let pos = f
                .get_at(Field::PlayerModDamageDonePos, i as u16)
                .unwrap_or(0) as i32;
            let neg = f
                .get_at(Field::PlayerModDamageDoneNeg, i as u16)
                .unwrap_or(0) as i32;
            pos.wrapping_add(neg)
        }),
        spell_healing: word(Field::PlayerModHealingDonePos),
        spell_penetration: (word(Field::PlayerModTargetResistance) as i32).saturating_neg(),
        shield_block: word(Field::PlayerShieldBlock),
        expertise: [
            word(Field::PlayerExpertise),
            word(Field::PlayerOffhandExpertise),
        ],
        pet_spell_power: word(Field::PlayerPetSpellPower) as i32,
        // No fabricated rating/primary-stat ratios while GameTables are unavailable.
        conversions: Default::default(),
    }
}

fn feed(
    script: Option<NonSendMut<UiScript>>,
    player: Query<&ObjectStore, With<SelfPlayer>>,
    tables: Option<Res<PaperDollTables>>,
) {
    let Some(mut script) = script else { return };
    script.set_wrath_paperdoll_stats(
        player
            .iter()
            .next()
            .map(|p| {
                let mut stats = snapshot(&p.0);
                if let Some(tables) = tables.as_ref() {
                    stats.conversions = conversions(&p.0, &tables.0);
                }
                stats
            })
            .unwrap_or_default(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_wrath_fields_feed_values_and_signed_school_bonuses() {
        let data = [
            (Field::PlayerCombatRating1, 24, 177),
            (Field::PlayerSpellCritPercentage1, 6, 12.5f32.to_bits()),
            (Field::PlayerCritPercentage, 0, 8.25f32.to_bits()),
            (Field::PlayerDodgePercentage, 0, 4.25f32.to_bits()),
            (Field::PlayerModDamageDonePos, 2, 100),
            (Field::PlayerModDamageDoneNeg, 2, (-20i32) as u32),
            (Field::PlayerModDamageDoneNeg, 3, (-30i32) as u32),
            (Field::PlayerModTargetResistance, 0, (-35i32) as u32),
            (Field::PlayerExpertise, 0, 12),
            (Field::PlayerOffhandExpertise, 0, 9),
            (Field::PlayerShieldBlock, 0, 234),
            (Field::PlayerModHealingDonePos, 0, 345),
        ];
        let pairs: Vec<_> = data
            .iter()
            .map(|&(field, offset, value)| (field.span().unwrap().index + offset, value))
            .collect();
        let stats = snapshot(&ObjectFields::from_pairs(&pairs));
        assert_eq!(stats.ratings[24], 177);
        assert_eq!(stats.spell_crit[6], 12.5);
        assert_eq!(stats.melee_crit, 8.25);
        assert_eq!(stats.dodge, 4.25);
        assert_eq!(stats.spell_damage[2..4], [80, -30], "signed, unclamped");
        assert_eq!(stats.spell_penetration, 35);
        assert_eq!(stats.expertise, [12, 9]);
        assert_eq!(stats.shield_block, 234);
        assert_eq!(stats.spell_healing, 345);
        assert_eq!(stats.conversions, Default::default());
    }
    #[test]
    fn feed_clears_logout_and_reseeds_a_new_vm() {
        let index = Field::PlayerCritPercentage.span().unwrap().index;
        let mut app = App::new();
        app.add_plugins(UiPaperDollPlugin);
        app.insert_non_send_resource(UiScript::new().unwrap());
        let player = app
            .world_mut()
            .spawn((
                SelfPlayer,
                ObjectStore(ObjectFields::from_pairs(&[(index, 8.25f32.to_bits())])),
            ))
            .id();
        app.update();
        assert_eq!(
            app.world()
                .non_send_resource::<UiScript>()
                .eval::<f64>("return GetCritChance()")
                .unwrap(),
            8.25
        );
        app.insert_non_send_resource(UiScript::new().unwrap());
        app.update();
        assert_eq!(
            app.world()
                .non_send_resource::<UiScript>()
                .eval::<f64>("return GetCritChance()")
                .unwrap(),
            8.25
        );
        app.world_mut().despawn(player);
        app.update();
        assert_eq!(
            app.world()
                .non_send_resource::<UiScript>()
                .eval::<f64>("return GetCritChance()")
                .unwrap(),
            0.0
        );
    }
    #[test]
    fn spirit_tick_split_and_missing_rows_are_not_fabricated() {
        assert_eq!(
            spirit_regen(40.0, Some(0.5), Some(0.2), Some(0.1)),
            (40.0, 4.0)
        );
        assert_eq!(
            spirit_regen(60.0, Some(0.5), Some(0.2), Some(0.1)),
            (54.0, 6.0)
        );
        assert_eq!(spirit_regen(60.0, None, Some(0.2), Some(0.1)), (0.0, 6.0));
        assert_eq!(spirit_regen(60.0, Some(0.5), None, None), (0.0, 0.0));
    }
    #[test]
    fn rating_conversion_requires_both_verified_rows_and_positive_divisor() {
        assert_eq!(rating_bonus(100, Some(0.5), Some(20.0)), 2.5);
        assert_eq!(rating_bonus(100, None, Some(20.0)), 0.0);
        assert_eq!(rating_bonus(100, Some(1.0), None), 0.0);
        assert_eq!(rating_bonus(100, Some(1.0), Some(0.0)), 0.0);
        assert_eq!(rating_bonus(100, Some(1.0), Some(f32::NAN)), 0.0);
    }
}
