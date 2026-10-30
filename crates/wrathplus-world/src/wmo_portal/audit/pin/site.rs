use super::*;

/// The subjects `WOW_PIN_SITE` names: every [`Site`] the harness carries.
const NAMED_SITES: &[(&str, Site)] = &[
    ("goldshire", GOLDSHIRE),
    ("fargodeep", FARGODEEP),
    ("deadmines", DEADMINES),
    ("blacksmith", BLACKSMITH),
    ("undercity", UNDERCITY),
    ("ironforge", IRONFORGE),
    ("shadowfang", SHADOWFANG),
    ("darnassus", DARNASSUS),
];

/// `WOW_PIN_SITE=<name>` picks a [`NAMED_SITES`] subject (default [`UNDERCITY`]), and
/// `WOW_PIN_WMO`, `_MAP`, `_TILE` and `_UID` each override one field of it.
pub(super) fn site_from_env() -> Site {
    let base = match std::env::var("WOW_PIN_SITE") {
        Ok(name) => {
            let key = name.trim().to_ascii_lowercase();
            *NAMED_SITES
                .iter()
                .find(|(n, _)| *n == key)
                .map(|(_, s)| s)
                .unwrap_or_else(|| {
                    let known: Vec<&str> = NAMED_SITES.iter().map(|(n, _)| *n).collect();
                    panic!("WOW_PIN_SITE={name:?} is not one of {known:?}")
                })
        }
        Err(_) => UNDERCITY,
    };
    Site {
        wmo: Box::leak(
            std::env::var("WOW_PIN_WMO")
                .unwrap_or_else(|_| base.wmo.to_string())
                .into_boxed_str(),
        ),
        map: Box::leak(
            std::env::var("WOW_PIN_MAP")
                .unwrap_or_else(|_| base.map.to_string())
                .into_boxed_str(),
        ),
        tile: std::env::var("WOW_PIN_TILE").map_or(base.tile, |s| {
            let (a, b) = s.split_once(',').expect("WOW_PIN_TILE wants tx,ty");
            (a.trim().parse().unwrap(), b.trim().parse().unwrap())
        }),
        uid: std::env::var("WOW_PIN_UID").map_or(base.uid, |s| s.trim().parse().unwrap()),
    }
}

pub(super) fn xyz(var: &str, default: [f32; 3]) -> [f32; 3] {
    let Ok(s) = std::env::var(var) else {
        return default;
    };
    let v: Vec<f32> = s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    <[f32; 3]>::try_from(&v[..]).unwrap_or_else(|_| panic!("{var} wants x,y,z; got {s:?}"))
}
