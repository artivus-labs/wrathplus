//! Title gender is separate from UnitSex, including self reached through another token.
use super::*;
use wrathplus_protocol::types::ObjectFields;

#[test]
fn own_title_gender_is_player_bytes_3_while_foreign_uses_unit_sex() {
    let fields = |gender| {
        ObjectStore(ObjectFields::from_pairs(&[
            (Field::ObjectGuid.index(), 42),
            (Field::ObjectGuid.index() + 1, 0),
            (Field::UnitBytes0.index(), 1 << 16), // UnitSex female
            (Field::PlayerBytes3.index(), gender),
        ]))
    };
    let names = NameCache::default();
    for gender in [0, 1, 2] {
        let own = fields(gender);
        let mut state = snapshot(&own, 42, None, 0, None, Default::default());
        enrich_unit(&mut state, 42, &names, &own, None, Some(&own));
        assert_eq!(state.sex, 3);
        assert_eq!(state.title_gender, Some(gender as u8));
        let mut foreign = snapshot(&own, 43, None, 0, None, Default::default());
        enrich_unit(&mut foreign, 43, &names, &own, None, Some(&own));
        assert_eq!(foreign.sex, 3);
        assert_eq!(foreign.title_gender, None);
    }
}
