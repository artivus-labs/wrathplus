use crate::script::UiScript;

#[test]
fn get_item_quality_color_is_the_references_own_table() {
    let s = UiScript::new().unwrap();
    let hex = |q: &str| {
        s.eval::<String>(&format!(
            "local r,g,b,h = GetItemQualityColor({q}) return h"
        ))
        .unwrap()
    };
    for (q, want) in [
        ("0", "|cff9d9d9d"),
        ("1", "|cffffffff"),
        ("2", "|cff1eff00"),
        ("3", "|cff0070dd"),
        ("4", "|cffa335ee"),
        ("5", "|cffff8000"),
        ("6", "|cffe6cc80"),
        ("7", "|cffe6cc80"),
        // Truncated, and a numeric string is a number.
        ("2.9", "|cff1eff00"),
        ("'3'", "|cff0070dd"),
    ] {
        assert_eq!(hex(q), want, "quality {q}");
    }

    // The floats are byte / 255: Epic's red is 0xa3.
    let r = s
        .eval::<f64>("local r = GetItemQualityColor(4) return r")
        .unwrap();
    assert!((r - 163.0 / 255.0).abs() < 1e-9, "epic red was {r}");

    // 8 and up answer Common (`0x61a516`); the compare is unsigned, so a negative does too.
    for q in ["8", "99", "-1", "-99"] {
        assert_eq!(hex(q), "|cffffffff", "quality {q}");
    }
    assert!(s.eval::<String>("return GetItemQualityColor(nil)").is_err());
}
