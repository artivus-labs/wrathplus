//! Generate native descriptor tables from protocol reference headers (no build dependencies).
//! Usage: rustc scripts/gen-fields.rs -o /tmp/azb-gen-fields && /tmp/azb-gen-fields \
//!   /path/to/AzerothCore/UpdateFields.h > fields/layout_generated.rs
//! Source: AzerothCore core/src/server/game/Entities/Object/Updates/UpdateFields.h
use std::{collections::BTreeMap, env, fs};

#[derive(Clone, Debug)]
struct Entry {
    index: u16,
    width: u16,
    class: String,
}
fn parse(path: &str) -> (BTreeMap<String, Entry>, BTreeMap<String, u16>) {
    let mut fields = BTreeMap::new();
    let mut symbols = BTreeMap::<String, u16>::new();
    let mut class = String::new();
    for line in fs::read_to_string(path).unwrap().lines() {
        if let Some(group) = line.trim().strip_prefix("enum E") {
            class = group.trim_end_matches("Fields").to_owned();
        }
        let Some((name, rest)) = line.trim().split_once('=') else {
            continue;
        };
        let name = name.trim();
        let expression = rest.split([',', '/']).next().unwrap().trim();
        let index: u16 = expression
            .split('+')
            .map(|s| {
                let s = s.trim();
                if let Some(hex) = s.strip_prefix("0x") {
                    u16::from_str_radix(hex, 16).unwrap()
                } else {
                    *symbols.get(s).unwrap()
                }
            })
            .sum();
        symbols.insert(name.to_owned(), index);
        let Some(size) = rest.split("Size: ").nth(1) else {
            continue;
        };
        let width = size
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let field_class = if name.starts_with("PLAYER_") {
            "Player"
        } else {
            &class
        };
        // Public PLAYER_BYTES[_2] and private PLAYER_FIELD_BYTES[2] are distinct.
        let name = match name {
            "PLAYER_FIELD_BYTES" => "PlayerFieldBytes".to_owned(),
            "PLAYER_FIELD_BYTES2" => "PlayerFieldBytes2".to_owned(),
            _ => name.replace("_FIELD_", "_"),
        };
        assert!(
            !fields.contains_key(&name),
            "duplicate normalized field: {}",
            name
        );
        fields.insert(
            name,
            Entry {
                index,
                width,
                class: field_class.to_owned(),
            },
        );
    }
    // Repeated descriptor groups have one semantic root. Offsets remain native dwords;
    // consumers of changed shapes (quest/visible item) use dialect-aware accessors.
    for (root, prefix, start, end) in [
        ("UnitPower", "UNIT_POWER", "UNIT_POWER1", "UNIT_MAXHEALTH"),
        (
            "UnitMaxPower",
            "UNIT_MAXPOWER",
            "UNIT_MAXPOWER1",
            "UNIT_LEVEL",
        ),
        (
            "PlayerQuestLog",
            "PLAYER_QUEST_LOG_",
            "PLAYER_QUEST_LOG_1_1",
            "PLAYER_VISIBLE_ITEM_1_CREATOR",
        ),
        (
            "PlayerVisibleItems",
            "PLAYER_VISIBLE_ITEM_",
            "PLAYER_VISIBLE_ITEM_1_CREATOR",
            "PLAYER_INV_SLOT_HEAD",
        ),
        (
            "ItemEnchantment",
            "ITEM_ENCHANTMENT",
            "ITEM_ENCHANTMENT",
            "ITEM_PROPERTY_SEED",
        ),
    ] {
        let start = fields
            .get(start)
            .or_else(|| {
                fields.get(match root {
                    "PlayerVisibleItems" => "PLAYER_VISIBLE_ITEM_1_ENTRYID",
                    "ItemEnchantment" => "ITEM_ENCHANTMENT_1_1",
                    _ => start,
                })
            })
            .cloned()
            .unwrap();
        let end_index = if root == "PlayerQuestLog" {
            fields
                .get(end)
                .or_else(|| fields.get("PLAYER_VISIBLE_ITEM_1_ENTRYID"))
                .unwrap()
                .index
        } else {
            fields.get(end).unwrap().index
        };
        // Wrath regen fields sit between max-power and level: only the native power slots.
        let width = if root == "UnitMaxPower" || root == "UnitPower" {
            fields
                .keys()
                .filter(|name| {
                    name.strip_prefix(prefix)
                        .is_some_and(|tail| tail.parse::<u8>().is_ok())
                })
                .count() as u16
        } else if root == "PlayerVisibleItems"
            && fields.contains_key("PLAYER_VISIBLE_ITEM_1_ENTRYID")
        {
            19 * 2
        } else {
            end_index - start.index
        };
        fields.retain(|name, _| {
            if root == "UnitPower" || root == "UnitMaxPower" {
                !name
                    .strip_prefix(prefix)
                    .is_some_and(|tail| tail.parse::<u8>().is_ok())
            } else {
                !name.starts_with(prefix)
            }
        });
        fields.insert(root.to_owned(), Entry { width, ..start });
    }
    let fields = fields
        .into_iter()
        .map(|(name, value)| {
            let name = if name.contains('_') {
                name.split('_')
                    .filter(|part| !part.is_empty())
                    .map(|part| {
                        let mut chars = part.chars();
                        match part {
                            "MAXHEALTH" => "MaxHealth".to_owned(),
                            "FACTIONTEMPLATE" => "FactionTemplate".to_owned(),
                            "DISPLAYID" => "DisplayId".to_owned(),
                            "NATIVEDISPLAYID" => "NativeDisplayId".to_owned(),
                            "MOUNTDISPLAYID" => "MountDisplayId".to_owned(),
                            "AURAFLAGS" => "AuraFlags".to_owned(),
                            "AURALEVELS" => "AuraLevels".to_owned(),
                            "AURAAPPLICATIONS" => "AuraApplications".to_owned(),
                            "AURASTATE" => "AuraState".to_owned(),
                            _ => format!(
                                "{}{}",
                                chars.next().unwrap(),
                                chars.as_str().to_ascii_lowercase()
                            ),
                        }
                    })
                    .collect::<String>()
            } else {
                name
            };
            (name, value)
        })
        .collect();
    (fields, symbols)
}
fn main() {
    let paths: Vec<_> = env::args().skip(1).collect();
    assert_eq!(paths.len(), 1, "expected the wrath reference header path");
    let (wrath, wends) = parse(&paths[0]);
    let names: Vec<_> = wrath.keys().cloned().collect();
    println!("// Generated by scripts/gen-fields.rs. Do not edit.\n// Wrath 12340: AzerothCore core/src/server/game/Entities/Object/Updates/UpdateFields.h\n// Input SHA-256 hash and regeneration: https://github.com/artivus-labs/wrathplus/wiki/DESCRIPTOR-SOURCES.\nuse super::{{FieldSpan, ObjectType}};\n\n#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]\n#[repr(u16)]\npub enum Field {{");
    for name in &names {
        println!("    {name},");
    }
    println!(
        "}}\nimpl Field {{\n    pub const ALL: [Self; {}] = [",
        names.len()
    );
    for name in &names {
        println!("        Self::{name},");
    }
    println!("    ];\n}}");
    for (label, fields, ends) in [("WRATH", wrath, wends)] {
        println!(
            "pub(super) const {label}: [Option<FieldSpan>; {}] = [",
            names.len()
        );
        for name in &names {
            if let Some(e) = fields.get(name) {
                println!("    Some(FieldSpan {{ index: {}, width: {}, object_type: ObjectType::{} }}), // {name}", e.index, e.width, e.class);
            } else {
                println!("    None, // {name}");
            }
        }
        println!("];");
        // Reverse tables include inherited blocks and preserve the forward enum's precedence.
        for class in [
            "Object",
            "Item",
            "Container",
            "Unit",
            "Player",
            "GameObject",
            "DynamicObject",
            "Corpse",
        ] {
            let len = ends[&format!("{}_END", class.to_ascii_uppercase())];
            let mut reverse = vec![None; usize::from(len)];
            for name in &names {
                let Some(e) = fields.get(name) else { continue };
                let inherited = e.class == "Object"
                    || (class == "Player" && e.class == "Unit")
                    || (class == "Container" && e.class == "Item");
                if e.class != class && !inherited {
                    continue;
                }
                for offset in 0..e.width {
                    let entry = &mut reverse[usize::from(e.index + offset)];
                    if entry.is_none() {
                        *entry = Some((name, offset));
                    }
                }
            }
            println!(
                "pub(super) const {label}_REVERSE_{}: [Option<(Field, u16)>; {len}] = [",
                class.to_ascii_uppercase()
            );
            for entry in reverse {
                match entry {
                    Some((name, offset)) => println!("    Some((Field::{name}, {offset})),"),
                    None => println!("    None,"),
                }
            }
            println!("];");
        }
        println!("pub(super) const {label}_REVERSE: [&[Option<(Field, u16)>]; 8] = [");
        for class in [
            "OBJECT",
            "ITEM",
            "CONTAINER",
            "UNIT",
            "PLAYER",
            "GAMEOBJECT",
            "DYNAMICOBJECT",
            "CORPSE",
        ] {
            println!("    &{label}_REVERSE_{class},");
        }
        println!("];");
        println!("pub(super) const {label}_ENDS: [u16; 8] = [");
        for class in [
            "OBJECT",
            "ITEM",
            "CONTAINER",
            "UNIT",
            "PLAYER",
            "GAMEOBJECT",
            "DYNAMICOBJECT",
            "CORPSE",
        ] {
            println!("    {},", ends[&format!("{class}_END")]);
        }
        println!("];");
    }
}
