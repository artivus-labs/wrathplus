//! Native quest reward lookup; formulas follow AzerothCore QuestDef.cpp:199,305.
use bevy::prelude::*;
use wrathplus_formats::{CharTitlesCatalog, QuestXpCatalog, TeamContributionPointsCatalog};

#[derive(Resource, Default)]
pub struct QuestRewardTables {
    titles: Option<CharTitlesCatalog>,
    xp: Option<QuestXpCatalog>,
    honor: Option<TeamContributionPointsCatalog>,
    /// `QuestFactionReward.dbc`, the reputation rewards' amounts.
    faction_rewards: Option<wrathplus_formats::QuestFactionRewardCatalog>,
}

pub(crate) fn load(mut commands: Commands, assets: Option<Res<wrathplus_assets::WorldAssets>>) {
    use wrathplus_assets::LockRecover;
    let Some(assets) = assets else { return };
    let mut chain = assets.chain.lock_recover();
    // Load independently: an absent table must not hide unrelated known rewards.
    commands.insert_resource(QuestRewardTables::read(&mut chain));
}

impl QuestRewardTables {
    fn read(chain: &mut wrathplus_formats::Chain) -> Self {
        Self {
            titles: wrathplus_formats::load_char_titles(chain).ok(),
            xp: wrathplus_formats::load_quest_xp(chain).ok(),
            honor: wrathplus_formats::load_team_contribution_points(chain).ok(),
            faction_rewards: wrathplus_formats::load_quest_faction_reward(chain).ok(),
        }
    }

    pub(crate) fn faction_rewards(&self) -> Option<&wrathplus_formats::QuestFactionRewardCatalog> {
        self.faction_rewards.as_ref()
    }

    pub(crate) fn fill_log(
        &self,
        detail: &mut wrathplus_ui::script::QuestLogDetail,
        native: &wrathplus_protocol::types::WrathQuestInfo,
        quest_level: i32,
        player_level: Option<u32>,
        subject: Option<&crate::npc_text::Subject>,
    ) {
        detail.reward_title = self.title(native.title_id, subject);
        detail.reward_xp = self
            .xp(quest_level, native.reward_xp_id, player_level)
            .unwrap_or(0);
        detail.reward_honor = self
            .honor(native.reward_honor, native.honor_multiplier, player_level)
            .unwrap_or(0);
    }

    pub(crate) fn title(
        &self,
        id: u32,
        subject: Option<&crate::npc_text::Subject>,
    ) -> Option<String> {
        let row = self.titles.as_ref()?.get(id)?;
        title_text(row, subject?)
    }

    /// The whole `CharTitles.dbc` catalog, if it loaded.
    pub(crate) fn char_titles(&self) -> Option<&CharTitlesCatalog> {
        self.titles.as_ref()
    }

    /// A title by its known-title bit, formatted for `subject` (`SMSG_TITLE_EARNED`).
    pub(crate) fn title_by_bit(
        &self,
        bit_index: u32,
        subject: &crate::npc_text::Subject,
    ) -> Option<String> {
        title_text(self.titles.as_ref()?.by_bit(bit_index)?, subject)
    }

    pub(crate) fn xp(
        &self,
        quest_level: i32,
        difficulty: u32,
        player_level: Option<u32>,
    ) -> Option<u32> {
        let player = player_level?;
        // A level -1 quest scales with the player (`0x5e0d7a`).
        let level = u32::try_from(quest_level).unwrap_or(player);
        let base = self.xp.as_ref()?.xp(level, difficulty as usize)?;
        Some(scale_xp(base, level, player))
    }

    pub(crate) fn honor(&self, addition: u32, multiplier: f32, level: Option<u32>) -> Option<u32> {
        if addition == 0 && multiplier <= 0.0 {
            return Some(0);
        }
        let value = self.honor.as_ref()?.get_clamped(level?)?;
        scaled_honor(value, multiplier, addition)
    }
}

/// The log's rewards at the player's level cap, `PLAYER_FIELD_MAX_LEVEL` (`[desc+0x11ac]`):
/// `GetQuestLogRewardMoney` answers the larger of the reward and the template's max-level money,
/// compared signed (`0x5e4928`-`0x5e4981`), and `GetQuestLogRewardXP` answers 0 (`0x5e4a21`).
pub(crate) fn at_level_cap(
    detail: &mut wrathplus_ui::script::QuestLogDetail,
    money_max_level: u32,
    at_cap: bool,
) {
    if at_cap {
        let max_level = i64::from(money_max_level as i32);
        detail.reward_money = i64::from(detail.reward_money).max(max_level) as u32;
        detail.reward_xp = 0;
    }
}

fn title_text(
    row: &wrathplus_formats::CharTitle,
    subject: &crate::npc_text::Subject,
) -> Option<String> {
    let text = if subject.gender == 1 {
        &row.female_name
    } else {
        &row.male_name
    };
    (!text.is_empty()).then(|| text.replace("%s", &subject.name))
}

fn scale_xp(base: u32, quest: u32, player: u32) -> u32 {
    let factor = (2 * (i64::from(quest) - i64::from(player)) + 20).clamp(1, 10) as u64;
    let xp = factor * u64::from(base) / 10;
    let (step, bias) = match xp {
        0..=100 => (5, 2),
        101..=500 => (10, 5),
        501..=1000 => (25, 12),
        _ => (50, 25),
    };
    (step * ((xp + bias) / step)).min(u64::from(u32::MAX)) as u32
}

fn scaled_honor(value: f32, multiplier: f32, addition: u32) -> Option<u32> {
    if !value.is_finite() || !multiplier.is_finite() || value < 0.0 || multiplier < 0.0 {
        return None;
    }
    // C++ multiplies the two floats before promotion by the double literal.
    let scaled = f64::from(value * multiplier) * 0.1000000014901161;
    (scaled < f64::from(u32::MAX))
        .then(|| (scaled as u32).checked_add(addition))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn dbc(rows: &[Vec<u32>], strings: &[u8]) -> Vec<u8> {
        let fields = rows[0].len() as u32;
        let mut bytes = b"WDBC".to_vec();
        for n in [rows.len() as u32, fields, fields * 4, strings.len() as u32] {
            bytes.extend_from_slice(&n.to_le_bytes());
        }
        for row in rows {
            for n in row {
                bytes.extend_from_slice(&n.to_le_bytes());
            }
        }
        bytes.extend_from_slice(strings);
        bytes
    }

    #[test]
    fn dbc_rewards_resolve_ids_buckets_dynamic_level_and_honor_clamp() {
        let root = std::env::temp_dir().join(format!("azb-wire-rewards-{}", std::process::id()));
        let db = root.join("DBFilesClient");
        std::fs::create_dir_all(&db).unwrap();
        let mut title = vec![0; 37];
        title[0] = 123;
        title[2] = 1;
        title[19] = 8;
        title[36] = 7;
        std::fs::write(
            db.join("CharTitles.dbc"),
            dbc(&[title], b"\0Sir %s\0Dame %s\0"),
        )
        .unwrap();
        let mut xp = vec![20; 11];
        xp[1] = 1000;
        xp[10] = 513;
        std::fs::write(db.join("QuestXP.dbc"), dbc(&[xp], b"\0")).unwrap();
        std::fs::write(
            db.join("TeamContributionPoints.dbc"),
            dbc(
                &[vec![20, 19.9f32.to_bits()], vec![100, 100.0f32.to_bits()]],
                b"\0",
            ),
        )
        .unwrap();
        let overrides = wrathplus_formats::AssetOverrides::from_roots([root.clone()]).unwrap();
        let mut chain = wrathplus_formats::Chain::from_overrides(
            overrides,
            wrathplus_formats::DataProfile::Wrath {
                locale: "enUS".into(),
            },
        );
        let tables = QuestRewardTables::read(&mut chain);
        let subject = crate::npc_text::Subject {
            name: "Ada".into(),
            race: 1,
            class: 1,
            gender: 1,
        };
        assert_eq!(
            tables.title(123, Some(&subject)).as_deref(),
            Some("Dame Ada")
        );
        assert_eq!(tables.title(7, Some(&subject)), None); // mask != title ID
        assert_eq!(tables.title(123, None), None);
        assert_eq!(tables.xp(-1, 9, Some(20)), Some(525));
        assert_eq!(tables.xp(20, 0, Some(26)), Some(800));
        assert_eq!(tables.xp(20, 10, Some(20)), None);
        assert_eq!(tables.xp(21, 0, Some(20)), None);
        assert_eq!(tables.xp(20, 0, None), None);
        assert_eq!(tables.honor(7, 1.0, Some(20)), Some(8));
        assert_eq!(tables.honor(7, 1.0, Some(101)), Some(17));
        assert_eq!(tables.honor(7, 1.0, Some(21)), None);
        assert_eq!(tables.honor(7, 1.0, None), None);
        let native = wrathplus_protocol::types::WrathQuestInfo {
            title_id: 123,
            reward_xp_id: 9,
            reward_honor: 7,
            honor_multiplier: 1.0,
            // Panel XP is not the query-derived award.
            reward_xp: 9999,
            ..Default::default()
        };
        let mut detail = wrathplus_ui::script::QuestLogDetail::default();
        tables.fill_log(&mut detail, &native, -1, Some(20), Some(&subject));
        assert_eq!((detail.reward_xp, detail.reward_honor), (525, 8));
        assert_eq!(detail.reward_title.as_deref(), Some("Dame Ada"));
        QuestRewardTables::default().fill_log(&mut detail, &native, 20, Some(20), Some(&subject));
        assert_eq!(
            (detail.reward_xp, detail.reward_honor, detail.reward_title),
            (0, 0, None)
        );
        // Missing XP does not hide titles or honor.
        std::fs::remove_file(db.join("QuestXP.dbc")).unwrap();
        let partial = QuestRewardTables::read(&mut chain);
        assert_eq!(partial.xp(20, 0, Some(20)), None);
        assert_eq!(
            partial.title(123, Some(&subject)).as_deref(),
            Some("Dame Ada")
        );
        assert_eq!(partial.honor(7, 1.0, Some(20)), Some(8));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_level_cap_pays_the_max_level_money_and_no_xp() {
        let mut detail = wrathplus_ui::script::QuestLogDetail {
            reward_money: 100,
            reward_xp: 500,
            ..Default::default()
        };
        at_level_cap(&mut detail, 250, false);
        assert_eq!((detail.reward_money, detail.reward_xp), (100, 500));
        at_level_cap(&mut detail, 250, true);
        assert_eq!((detail.reward_money, detail.reward_xp), (250, 0));
        // Signed: a max-level money of -1 loses to the reward (`0x5e497d`).
        at_level_cap(&mut detail, u32::MAX, true);
        assert_eq!(detail.reward_money, 250);
    }
    #[test]
    fn title_uses_actual_gender_and_formats_player_name() {
        let mut row = wrathplus_formats::CharTitle {
            id: 123,
            mask_id: 7,
            male_name: "Sir %s".into(),
            female_name: "Dame %s".into(),
        };
        let mut subject = crate::npc_text::Subject {
            name: "Ada".into(),
            race: 1,
            class: 1,
            gender: 1,
        };
        assert_eq!(title_text(&row, &subject).as_deref(), Some("Dame Ada"));
        subject.gender = 0;
        assert_eq!(title_text(&row, &subject).as_deref(), Some("Sir Ada"));
        row.male_name.clear();
        assert_eq!(title_text(&row, &subject), None);
    }
    #[test]
    fn xp_level_scaling_and_rounding() {
        for (base, expected) in [
            (98, 100),
            (104, 100),
            (106, 110),
            (511, 500),
            (513, 525),
            (1025, 1050),
        ] {
            assert_eq!(scale_xp(base, 20, 20), expected);
        }
        assert_eq!(scale_xp(1000, 20, 26), 800);
        assert_eq!(scale_xp(1000, 20, 40), 100);
        assert_eq!(scale_xp(1000, 40, 20), 1000);
    }
    #[test]
    fn honor_truncates_before_addition_and_rejects_invalid_values() {
        assert_eq!(scaled_honor(19.9, 1.0, 7), Some(8));
        assert_eq!(scaled_honor(100.0, 2.0, 7), Some(27));
        assert_eq!(scaled_honor(f32::NAN, 1.0, 7), None);
    }
    #[test]
    fn absent_tables_do_not_guess_rewards() {
        let tables = QuestRewardTables::default();
        assert_eq!(tables.xp(20, 0, Some(20)), None);
        assert_eq!(tables.honor(7, 0.0, Some(20)), None);
        assert_eq!(tables.honor(0, 0.0, Some(20)), Some(0));
        assert_eq!(tables.title(1, None), None);
    }
}
