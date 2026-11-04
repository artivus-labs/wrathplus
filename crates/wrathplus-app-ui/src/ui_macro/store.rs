//! The macro file format, the reference's own: a `MACRO %d "%s" %s` header carrying the macro's
//! id, the body lines and a bare `END`, in `macros-cache.txt`, one file per scope. WrathPlus ends
//! lines with `\n` where the reference writes `\r\n`, and reads either. A name holding a `"` runs
//! from the first quote to the last, since the reference's `"%s"` is unescaped.

use wrathplus_ui::script::{MacroView, MAX_ACCOUNT_MACROS};

/// The path prefix the client prepends to a stored icon basename (`[0xace1e8]`, `0x566ac0`).
pub(super) const ICON_PREFIX: &str = "Interface\\Icons\\";

/// Parse one scope's file as build 12340's loader reads it (`0x567c10`): at most 36 records for
/// either scope; an id of 0, a repeated id or an empty name loses its record; and an id below the
/// scope's base (`base`, `0xa0f834`) gains it, so a file numbered from 1, as WrathPlus's 1.12-era
/// character files are, loads under the character ids. File order is kept; the engine sorts.
pub(super) fn parse(text: &str, base: u32) -> Vec<MacroView> {
    let mut out: Vec<MacroView> = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(rest) = line.trim_start().strip_prefix("MACRO ") else {
            continue; // stray text between records
        };
        let header = parse_header(rest);
        // The body runs to a bare END, or to the next MACRO or EOF, so a truncated file keeps its
        // last macro.
        let mut body: Vec<&str> = Vec::new();
        loop {
            match lines.peek() {
                None => break,
                Some(l) if l.trim() == "END" => {
                    lines.next();
                    break;
                }
                Some(l) if l.trim_start().starts_with("MACRO ") => break,
                Some(_) => body.push(lines.next().unwrap_or_default()),
            }
        }
        let Some((id, name, icon)) = header else {
            continue;
        };
        let id = if id < base { id + base } else { id };
        if id == 0
            || name.is_empty()
            || out.len() >= MAX_ACCOUNT_MACROS
            || out.iter().any(|m| m.id == id)
        {
            continue;
        }
        out.push(MacroView {
            id,
            name,
            texture: icon.map(icon_path),
            body: body.join("\n"),
        });
    }
    out
}

/// The tail of a `MACRO ` header line: `<index> "<name>" <icon>`.
fn parse_header(rest: &str) -> Option<(u32, String, Option<String>)> {
    let (index, rest) = rest.split_once('"')?;
    let index: u32 = index.trim().parse().ok()?;
    // To the last quote on the line: the reference's `"%s"` is unescaped.
    let close = rest.rfind('"')?;
    let name = rest[..close].to_string();
    let icon = rest[close + 1..].trim();
    Some((index, name, (!icon.is_empty()).then(|| icon.to_string())))
}

/// Serialize a scope's macros back into the reference's format, each under its id.
pub(super) fn write(macros: &[MacroView]) -> String {
    let mut out = String::new();
    for m in macros {
        let icon = m.texture.as_deref().map(icon_token).unwrap_or_default();
        out.push_str(&format!("MACRO {} \"{}\" {}\n", m.id, m.name, icon));
        if !m.body.is_empty() {
            out.push_str(&m.body);
            out.push('\n');
        }
        out.push_str("END\n");
    }
    out
}

/// A stored icon token to its texture path; a token with a path separator is taken whole.
fn icon_path(token: String) -> String {
    if token.contains('\\') || token.contains('/') {
        token
    } else {
        format!("{ICON_PREFIX}{token}")
    }
}

/// A texture path to its stored token: the basename in the icons folder, else the whole path.
fn icon_token(path: &str) -> &str {
    path.strip_prefix(ICON_PREFIX).unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrathplus_ui::script::CHARACTER_MACRO_BASE;

    /// A real `macros-cache.txt`, transcribed with `\n` line ends; its ids are out of order.
    const REAL_FILE: &str = "MACRO 1 \"f\" Ability_Ambush\n\
         .cheat fly on\n\
         .modify aspeed 10\n\
         END\n\
         MACRO 3 \"ns\" Ability_BackStab\n\
         .cheat fly off\n\
         .modify aspeed 1\n\
         END\n\
         MACRO 2 \"w\" Ability_Creature_Cursed_05\n\
         .wchange 0 0\n\
         END\n";

    #[test]
    fn a_real_macros_cache_file_parses_with_its_ids() {
        let macros = parse(REAL_FILE, 0);
        let rows: Vec<(u32, &str)> = macros.iter().map(|m| (m.id, m.name.as_str())).collect();
        assert_eq!(
            rows,
            [(1, "f"), (3, "ns"), (2, "w")],
            "file order, each under its id"
        );
        assert_eq!(
            macros[0].texture.as_deref(),
            Some("Interface\\Icons\\Ability_Ambush"),
            "the stored token is a basename; the client's own prefix completes it"
        );
        assert_eq!(macros[0].body, ".cheat fly on\n.modify aspeed 10");
        assert_eq!(macros[2].body, ".wchange 0 0");
    }

    /// WrathPlus's 1.12-era character files number from 1; the loader lifts an id under the base
    /// (`0x567c10`), so they load as character ids, and the write keeps them there.
    #[test]
    fn a_character_file_numbered_from_one_loads_under_the_character_base() {
        let macros = parse(REAL_FILE, CHARACTER_MACRO_BASE);
        assert_eq!(macros[0].id, CHARACTER_MACRO_BASE + 1);
        assert_eq!(macros[1].id, CHARACTER_MACRO_BASE + 3);
        let text = write(&macros);
        assert!(text.starts_with(&format!("MACRO {} \"f\"", CHARACTER_MACRO_BASE + 1)));
        assert_eq!(parse(&text, CHARACTER_MACRO_BASE), macros, "round trip");
    }

    #[test]
    fn write_then_parse_round_trips_and_matches_the_reference_shape() {
        let macros = parse(REAL_FILE, 0);
        let text = write(&macros);
        assert!(text.starts_with("MACRO 1 \"f\" Ability_Ambush\n"));
        assert!(text.contains("\nMACRO 2 \"w\" Ability_Creature_Cursed_05\n"));
        assert!(text.ends_with("END\n"));
        assert_eq!(parse(&text, 0), macros, "round trip");
    }

    #[test]
    fn an_empty_body_and_a_missing_icon_survive_the_round_trip() {
        let m = vec![MacroView {
            id: 4,
            name: "bare".into(),
            texture: None,
            body: String::new(),
        }];
        let text = write(&m);
        assert_eq!(text, "MACRO 4 \"bare\" \nEND\n");
        assert_eq!(parse(&text, 0), m);
    }

    /// A file truncated before its last `END` still yields that macro; a bad header, id 0, a
    /// repeated id and an empty name each lose their record.
    #[test]
    fn malformed_records_lose_only_themselves() {
        let text = "garbage line\n\
             MACRO notanumber \"x\" Icon\n\
             END\n\
             MACRO 0 \"zero\" Icon\n\
             END\n\
             MACRO 2 \"good\" Ability_Ambush\n\
             /say hi\n\
             END\n\
             MACRO 2 \"again\" Ability_Ambush\n\
             END\n\
             MACRO 5 \"\" Ability_Ambush\n\
             END\n\
             MACRO 3 \"truncated\" Ability_Ambush\n\
             /say bye\n";
        let macros = parse(text, 0);
        let names: Vec<&str> = macros.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["good", "truncated"]);
        assert_eq!(macros[1].body, "/say bye", "no END is still a macro");
    }

    #[test]
    fn a_quoted_name_round_trips_the_way_the_reference_would_write_it() {
        let m = vec![MacroView {
            id: 1,
            name: "say \"hi\"".into(),
            texture: Some("Interface\\Icons\\Ability_Ambush".into()),
            body: "/say hi".into(),
        }];
        assert_eq!(parse(&write(&m), 0), m);
    }

    #[test]
    fn a_full_path_icon_token_is_taken_whole() {
        let text = "MACRO 1 \"custom\" Interface\\Buttons\\UI-Panel-Button\nEND\n";
        let macros = parse(text, 0);
        assert_eq!(
            macros[0].texture.as_deref(),
            Some("Interface\\Buttons\\UI-Panel-Button")
        );
        assert_eq!(write(&macros), text, "and writes back unchanged");
    }

    /// The loader stops at 36 records for either scope (`0x567c10`).
    #[test]
    fn a_file_longer_than_36_records_is_truncated() {
        let text: String = (1..=40)
            .map(|i| format!("MACRO {i} \"m{i}\" Ability_Ambush\nEND\n"))
            .collect();
        assert_eq!(parse(&text, 0).len(), MAX_ACCOUNT_MACROS);
    }
}
