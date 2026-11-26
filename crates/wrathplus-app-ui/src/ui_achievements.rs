//! Wrath static achievement metadata. This is not a player-progress store.
use bevy::prelude::*;
use wrathplus_assets::{AssetSet, LockRecover, WorldAssets};
use wrathplus_ui::script::{
    UiScript, WrathAchievement, WrathAchievementCategory, WrathAchievementCriterion,
    WrathAchievementMetadata,
};

#[derive(Resource)]
pub(crate) struct AchievementMetadata(WrathAchievementMetadata);

impl AchievementMetadata {
    /// An `Achievement.dbc` row's name, for the chat's `$a` link.
    pub(crate) fn name(&self, id: u32) -> Option<&str> {
        self.0.achievements.get(&id).map(|a| a.name.as_str())
    }

    /// An `Achievement.dbc` row's `Order`, the earned list's same-minute tie-break.
    pub(crate) fn order(&self, id: u32) -> Option<u32> {
        self.0.achievements.get(&id).map(|a| a.order)
    }
}

pub struct UiAchievementsPlugin;
impl Plugin for UiAchievementsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load.after(AssetSet::Open));
    }
}

fn load(mut commands: Commands, assets: Option<Res<WorldAssets>>) {
    let Some(assets) = assets else { return };
    let mut chain = assets.chain.lock_recover();
    if !matches!(
        chain.profile(),
        wrathplus_formats::DataProfile::Wrath { .. }
    ) {
        return;
    }
    if let Some(metadata) = metadata_from(&mut chain) {
        commands.insert_resource(AchievementMetadata(metadata));
    }
}

/// The static achievement catalogs as the VM reads them, from the client's DBCs; `None` without
/// Achievement.dbc or Achievement_Criteria.dbc.
pub(crate) fn metadata_from(
    chain: &mut wrathplus_formats::Chain,
) -> Option<WrathAchievementMetadata> {
    use wrathplus_formats::achievements::{
        load_achievement_categories, load_achievement_criteria, load_achievement_icons,
        load_achievements,
    };
    // Icons and categories are optional: without them the window shows no icons or categories.
    let icons = load_achievement_icons(chain).unwrap_or_else(|e| {
        warn!("achievement icons unavailable: {e:#}");
        Default::default()
    });
    let categories = load_achievement_categories(chain).map_err(|e| {
        warn!("achievement categories unavailable: {e:#}");
    });
    match (load_achievements(chain), load_achievement_criteria(chain)) {
        (Ok(achievements), Ok(criteria)) => {
            let mut metadata = WrathAchievementMetadata::default();
            if let Ok(categories) = categories {
                // The catalog is keyed by id; the window lists by the file's `order` column.
                let mut rows: Vec<_> = categories.iter().collect();
                rows.sort_by_key(|c| (c.order, c.id));
                metadata.categories = rows
                    .into_iter()
                    .map(|c| WrathAchievementCategory {
                        id: c.id,
                        parent_id: c.parent_id,
                        name: c.name.clone(),
                    })
                    .collect();
            }
            for row in achievements.iter() {
                metadata.achievements.insert(
                    row.id,
                    WrathAchievement {
                        id: row.id,
                        name: row.name.clone(),
                        category_id: row.category_id,
                        flags: row.flags,
                        required_criteria_count: row.required_criteria_count,
                        reference_achievement_id: row.reference_achievement_id,
                        required_faction: row.required_faction,
                        parent_achievement_id: row.parent_achievement_id,
                        description: row.description.clone(),
                        points: row.points,
                        icon: icons.get(&row.icon_id).cloned().unwrap_or_default(),
                        reward: row.reward.clone(),
                        order: row.order,
                    },
                );
                metadata.linked_criteria.insert(
                    row.id,
                    criteria.for_achievement(row.id).map(|row| row.id).collect(),
                );
            }
            for row in criteria.iter() {
                metadata.criteria.insert(
                    row.id,
                    WrathAchievementCriterion {
                        id: row.id,
                        achievement_id: row.achievement_id,
                        criteria_type: row.criteria_type,
                        asset_id: row.asset_id,
                        required_quantity: row.required_quantity,
                        description: row.description.clone(),
                        flags: row.flags,
                        time_limit: row.time_limit_seconds,
                    },
                );
            }
            Some(metadata)
        }
        (achievements, criteria) => {
            warn!(
                "achievement metadata unavailable: achievements={:?}, criteria={:?}",
                achievements.err(),
                criteria.err()
            );
            None
        }
    }
}

/// One startup handoff for each fresh in-game VM, before any FrameXML executes.
pub(crate) fn seed(world: &World, script: &mut UiScript) {
    if let Some(metadata) = world.get_resource::<AchievementMetadata>() {
        script.set_wrath_achievement_metadata(metadata.0.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_snapshot_is_available_to_each_fresh_vm_before_ui_execution() {
        let mut world = World::new();
        let mut metadata = WrathAchievementMetadata::default();
        metadata.achievements.insert(
            1,
            WrathAchievement {
                id: 1,
                name: "Metadata only".into(),
                category_id: 42,
                flags: 0,
                required_criteria_count: 0,
                reference_achievement_id: 0,
                ..Default::default()
            },
        );
        for id in [12, 10] {
            metadata.criteria.insert(
                id,
                WrathAchievementCriterion {
                    id,
                    achievement_id: 1,
                    criteria_type: 0,
                    asset_id: 7,
                    required_quantity: 8,
                    description: format!("criterion {id}"),
                    ..Default::default()
                },
            );
        }
        // Preserve raw linked order, not the criterion-ID map's sorted order.
        metadata.linked_criteria.insert(1, vec![12, 10]);
        world.insert_resource(AchievementMetadata(metadata));
        for _ in 0..2 {
            let mut script = UiScript::new().unwrap();
            seed(&world, &mut script);
            script.run("assert(GetAchievementCategory(1)==42); assert(GetAchievementNumCriteria(1)==2); local d,t,c,q,r,n,f,a,s,id=GetAchievementCriteriaInfo(1,1); assert(id==12 and c==false and q==0 and r==8 and n==nil)").unwrap();
        }
    }

    #[test]
    fn absent_catalog_is_not_replaced_with_fabricated_metadata() {
        let world = World::new();
        let mut script = UiScript::new().unwrap();
        seed(&world, &mut script);
        script.run("assert(select('#',GetAchievementCategory(1))==0); assert(select('#',GetAchievementCriteriaInfo(1,1))==0)").unwrap();
    }
}
