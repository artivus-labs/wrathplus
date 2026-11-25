use super::*;

fn dbc(rows: &[Vec<u32>], strings: &[u8]) -> Vec<u8> {
    let fields = rows[0].len() as u32;
    let mut b = b"WDBC".to_vec();
    for v in [rows.len() as u32, fields, fields * 4, strings.len() as u32] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for row in rows {
        for v in row {
            b.extend_from_slice(&v.to_le_bytes());
        }
    }
    b.extend_from_slice(strings);
    b
}

#[test]
fn achievement_preserves_raw_fields_and_selected_locale() {
    let mut r: Vec<u32> = (0..62).collect();
    r[0] = 95;
    r[1] = u32::MAX;
    r[2] = u32::MAX;
    for base in [4, 21, 43] {
        r[base..base + 16].fill(0);
        r[base] = 1;
        r[base + 2] = 9;
    }
    let mut later = r.clone();
    later[0] = 7;
    let b = dbc(&[r, later], b"\0English\0Francais\0");
    let c = achievements(&b, "enUS").unwrap();
    let a = c.get(95).unwrap();
    assert_eq!((a.required_faction, a.map_id), (-1, -1));
    assert_eq!(
        (a.parent_achievement_id, a.reference_achievement_id),
        (3, 61)
    );
    assert_eq!(
        (
            a.category_id,
            a.points,
            a.order,
            a.flags,
            a.icon_id,
            a.required_criteria_count
        ),
        (38, 39, 40, 41, 42, 60)
    );
    assert_eq!(
        (&*a.name, &*a.description, &*a.reward),
        ("English", "English", "English")
    );
    let fr = achievements(&b, "frFR").unwrap();
    assert_eq!(fr.get(95).unwrap().name, "Francais");
    assert_eq!(fr.get(95).unwrap().description, "Francais");
    assert_eq!(fr.get(95).unwrap().reward, "Francais");
    assert_eq!(achievements(&b, "deDE").unwrap().get(95).unwrap().name, "");
    assert_eq!(c.len(), 2);
    assert!(!c.is_empty());
    assert_eq!(
        c.iter().map(|a| a.id).collect::<Vec<_>>(),
        [95, 7],
        "file order"
    );
    assert!(c.get(999).is_none());
}

#[test]
fn criteria_keep_hidden_rows_raw_requirements_and_file_order() {
    let mut r: Vec<u32> = (0..31).collect();
    r[0] = 200;
    r[1] = 9999;
    r[9..25].fill(0);
    r[9] = 1;
    r[11] = 9;
    r[26] = 0x80000002;
    let mut second = r.clone();
    second[0] = 100;
    second[30] = 0;
    let b = dbc(&[r, second], b"\0English\0Francais\0");
    let c = criteria(&b, "enUS").unwrap();
    let a = c.get(200).unwrap();
    assert_eq!(
        (
            a.achievement_id,
            a.criteria_type,
            a.asset_id,
            a.required_quantity
        ),
        (9999, 2, 3, 4)
    );
    assert_eq!(a.additional_requirements, [(5, 6), (7, 8)]);
    assert_eq!(
        (
            a.flags,
            a.timed_type,
            a.timer_start_event,
            a.time_limit_seconds,
            a.order
        ),
        (0x80000002, 27, 28, 29, 30)
    );
    assert_eq!(a.description, "English");
    assert_eq!(
        criteria(&b, "frFR").unwrap().get(200).unwrap().description,
        "Francais"
    );
    assert_eq!(
        criteria(&b, "deDE").unwrap().get(200).unwrap().description,
        ""
    );
    assert_eq!(
        c.for_achievement(9999).map(|r| r.id).collect::<Vec<_>>(),
        [200, 100]
    );
    assert_eq!(c.for_achievement(200).count(), 0);
    assert!(c.get(9999).is_none());
    assert_eq!(c.len(), 2);
    assert!(!c.is_empty());
    assert_eq!(c.iter().count(), 2);
}

#[test]
fn category_root_and_unresolved_parent_preserved() {
    let mut r = vec![0; 20];
    r[0] = 1;
    r[1] = u32::MAX;
    r[2] = 1;
    r[4] = 9;
    r[19] = 10;
    let mut second = r.clone();
    second[0] = 2;
    second[1] = 999;
    let b = dbc(&[r, second], b"\0English\0Francais\0");
    let c = categories(&b, "enUS").unwrap();
    assert_eq!(c.get(1).unwrap().parent_id, -1);
    assert_eq!(c.get(1).unwrap().order, 10);
    assert_eq!(c.get(2).unwrap().parent_id, 999);
    assert!(c.get(999).is_none());
    assert_eq!(
        categories(&b, "frFR").unwrap().get(1).unwrap().name,
        "Francais"
    );
    assert_eq!(categories(&b, "deDE").unwrap().get(1).unwrap().name, "");
    assert_eq!(c.len(), 2);
    assert!(!c.is_empty());
    assert_eq!(c.iter().count(), 2);
}

#[test]
fn layouts_and_locale_fail_closed() {
    assert!(achievements(&dbc(&[vec![0; 61]], b"\0"), "enUS").is_err());
    assert!(criteria(&dbc(&[vec![0; 30]], b"\0"), "enUS").is_err());
    assert!(categories(&dbc(&[vec![0; 19]], b"\0"), "enUS").is_err());
    assert!(achievements(&dbc(&[vec![0; 62]], b"\0"), "invalid").is_err());
    assert!(criteria(&dbc(&[vec![0; 31]], b"\0"), "invalid").is_err());
    assert!(categories(&dbc(&[vec![0; 20]], b"\0"), "invalid").is_err());
}
