use super::*;

#[test]
fn is_harmful_follows_the_client_classifier() {
    let mut d = SpellDisplay::default();
    assert!(!d.is_harmful(), "an empty row targets nobody");
    d.targets = 0x80;
    assert!(d.is_harmful(), "the enemy target flag alone is harmful");
    d.targets = 0x80 | 0x100;
    assert!(!d.is_harmful(), "the ally flag wins over the enemy flag");
    d.targets = 0;
    d.effect_implicit_target_a = [6, 0, 0];
    assert!(d.is_harmful(), "TARGET_UNIT_TARGET_ENEMY in slot 0");
    d.effect_implicit_target_a = [21, 0, 0];
    assert!(!d.is_harmful(), "a single-friend target is not harmful");
    d.effect_implicit_target_b = [0, 16, 0];
    assert!(d.is_harmful(), "an enemy area in a B slot counts too");
    d.effect_implicit_target_b = [0, 0, 0];
    d.effect_implicit_target_a = [22, 0, 54];
    assert!(d.is_harmful(), "the enemy cone in the third effect");
    d.targets = 0x100;
    assert!(!d.is_harmful(), "the ally flag short-circuits the walk");
    assert!(d.is_helpful(), "and is helpful");
    d.targets = 0;
    d.effect_implicit_target_a = [21, 0, 0];
    d.effect_implicit_target_b = [0, 93, 0];
    assert!(
        d.is_harmful() && !d.is_helpful(),
        "an enemy corpse area beats a friend target"
    );
    d.effect_implicit_target_b = [0, 0, 0];
    assert!(d.is_helpful(), "TARGET_UNIT_TARGET_ALLY alone is helpful");
    d.effect_implicit_target_a = [1, 0, 0];
    d.effect_apply_aura = [4, 0, 0];
    assert!(
        !d.is_helpful() && !d.is_harmful(),
        "a self dummy aura is neither"
    );
    d.effect_apply_aura = [0, 0, 0];
    assert!(
        d.is_helpful(),
        "a self effect without the dummy aura is helpful"
    );
}

#[test]
fn learn_announcement_reads_the_three_attribute_bits_in_the_clients_order() {
    let mut d = SpellDisplay::default();
    assert_eq!(
        d.learn_announcement(),
        Some(LearnAnnouncement::Spell),
        "a plain row is a spell — Fireball 133 carries Attributes 0x10000"
    );
    d.attributes = 0x10;
    assert_eq!(
        d.learn_announcement(),
        Some(LearnAnnouncement::Ability),
        "SPELL_ATTR_ABILITY picks the ability wording"
    );
    d.attributes = 0x20;
    assert_eq!(
        d.learn_announcement(),
        Some(LearnAnnouncement::Recipe),
        "SPELL_ATTR_IS_TRADESKILL diverts to the recipe line"
    );
    d.attributes = 0x20 | 0x10;
    assert_eq!(
        d.learn_announcement(),
        Some(LearnAnnouncement::Recipe),
        "the tradeskill branch is taken before the ability bit is ever read"
    );
    d.attributes = 0x80;
    assert_eq!(
        d.learn_announcement(),
        None,
        "SPELL_ATTR_DO_NOT_DISPLAY is announced silently — every language and proficiency"
    );
    d.attributes = 0x80 | 0x20;
    assert_eq!(
        d.learn_announcement(),
        None,
        "…and the sign test at 0x4b290f runs before the tradeskill test at 0x4b2917"
    );
    d.attributes = 0x40;
    assert_eq!(
        d.learn_announcement(),
        Some(LearnAnnouncement::Spell),
        "PASSIVE is not read by the block at all — only DO_NOT_DISPLAY silences a spell"
    );
}

#[test]
fn announces_unlearn_is_not_the_mirror_of_the_learn_gates() {
    let mut d = SpellDisplay::default();
    assert!(d.announces_unlearn(), "a plain row says it");
    d.attributes = 0x10;
    assert!(
        d.announces_unlearn(),
        "ABILITY is a learn-wording bit and nothing to this path"
    );
    d.attributes = 0x40;
    assert!(d.announces_unlearn(), "PASSIVE is not read here either");
    d.attributes = 0x20;
    assert!(
        !d.announces_unlearn(),
        "IS_TRADESKILL: 0x5ea170 zeroes the flag"
    );
    d.attributes = 0x80;
    assert!(!d.announces_unlearn(), "DO_NOT_DISPLAY: 0x5ea29b");
    d.attributes = 0;
    d.cast_ui = 1;
    assert!(
        !d.announces_unlearn(),
        "castUI > 0 never reaches 0x5ea292 — its single entry is 0x5ea17a jle"
    );
    assert_eq!(
        d.learn_announcement(),
        Some(LearnAnnouncement::Spell),
        "…while the SAME row still announces its learn: castUI is read after the message"
    );
}

/// The reference tests the subtext's first byte (`0x4b2963`), so an empty rank is no rank.
#[test]
fn ranked_name_appends_the_subtext_only_when_there_is_one() {
    let mut d = SpellDisplay {
        name: "Fireball".to_string(),
        ..SpellDisplay::default()
    };
    assert_eq!(d.ranked_name(), "Fireball", "no subtext, no parentheses");
    d.rank = Some(String::new());
    assert_eq!(
        d.ranked_name(),
        "Fireball",
        "an empty subtext is no subtext"
    );
    d.rank = Some("Rank 2".to_string());
    assert_eq!(d.ranked_name(), "Fireball (Rank 2)");
}
