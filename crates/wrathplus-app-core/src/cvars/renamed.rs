//! CVars a saved `config.toml` may still hold under 1.12's names, which build 12340 replaced with
//! `Sound_*` (`https://github.com/artivus-labs/wrathplus/wiki/WRATH-CVARS`). Each is carried to its new name at load, unless the file
//! already holds the new one, and the next save writes only the new name.

use std::collections::BTreeMap;

/// 1.12's name, then build 12340's.
pub(super) const RENAMED: &[(&str, &str)] = &[
    ("MasterVolume", "Sound_MasterVolume"),
    ("SoundVolume", "Sound_SFXVolume"),
    ("MusicVolume", "Sound_MusicVolume"),
    ("AmbienceVolume", "Sound_AmbienceVolume"),
    ("MasterSoundEffects", "Sound_EnableAllSound"),
    ("EnableMusic", "Sound_EnableMusic"),
    ("EnableAmbience", "Sound_EnableAmbience"),
    ("EnableErrorSpeech", "Sound_EnableErrorSpeech"),
    ("SoundReverb", "Sound_EnableReverb"),
    ("SoundListenerAtCharacter", "Sound_ListenerAtCharacter"),
    ("EmoteSounds", "Sound_EnableEmoteSounds"),
    ("SoundZoneMusicNoDelay", "Sound_ZoneMusicNoDelay"),
];

/// The file's `[cvars]` with every 1.12 name carried to its 12340 name, matched ignoring case as
/// the registry does; a new name already present wins and the old one is dropped.
pub(super) fn migrate(mut table: BTreeMap<String, String>) -> BTreeMap<String, String> {
    for (old, new) in RENAMED {
        let Some(key) = table.keys().find(|k| k.eq_ignore_ascii_case(old)).cloned() else {
            continue;
        };
        let value = table.remove(&key).expect("just found");
        if !table.keys().any(|k| k.eq_ignore_ascii_case(new)) {
            table.insert((*new).to_string(), value);
        }
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_1_12_name_moves_to_its_12340_name_and_a_new_one_wins() {
        let file: BTreeMap<String, String> = [
            ("mastervolume", "0.3"),
            ("MusicVolume", "0.1"),
            ("Sound_MusicVolume", "0.9"),
            ("gxResolution", "1600x900"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let got = migrate(file);
        let pairs: Vec<(&str, &str)> = got.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(
            pairs,
            [
                ("Sound_MasterVolume", "0.3"),
                ("Sound_MusicVolume", "0.9"),
                ("gxResolution", "1600x900"),
            ]
        );
    }
}
