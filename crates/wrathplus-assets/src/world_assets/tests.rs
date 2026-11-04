use super::*;

#[test]
fn loose_addon_file_maps_addon_paths_case_insensitively() {
    let root = std::env::temp_dir().join(format!(
        "wrathplus-loose-sprite-test-{}",
        std::process::id()
    ));
    let dir = root.join("Atlas").join("Images").join("Maps");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("BlackrockDepths.blp"), b"x").unwrap();

    // The returned spelling depends on the filesystem's case sensitivity, so check the file.
    let hit = loose_addon_file(
        &root,
        "interface\\addons\\atlas\\images\\maps\\blackrockdepths.blp",
    )
    .expect("mixed-case tree resolves a lowercase candidate");
    assert_eq!(std::fs::read(&hit).unwrap(), b"x");

    assert_eq!(loose_addon_file(&root, "interface\\icons\\foo.blp"), None);
    // A directory is not a file, and a missing file is a miss, not an error.
    assert_eq!(loose_addon_file(&root, "interface\\addons\\atlas"), None);
    assert_eq!(
        loose_addon_file(&root, "interface\\addons\\atlas\\images\\maps\\nope.blp"),
        None
    );
    // Dot-components never reach the filesystem; `normalize_path` already leaves no `/`.
    assert_eq!(
        loose_addon_file(&root, "interface\\addons\\..\\..\\etc\\passwd"),
        None
    );

    std::fs::remove_dir_all(&root).ok();
}

/// Against the real chain: a client file must still come from the archive, and the loose
/// folder is a fallback, not a shadow over it.
#[test]
fn read_chain_or_loose_asks_the_chain_then_an_addons_own_file_on_disk() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let chain = Mutex::new(Chain::open(&data).expect("open the chain"));
    let root =
        std::env::temp_dir().join(format!("wrathplus-loose-audio-test-{}", std::process::id()));
    let addons = root.join("AddOns");
    std::fs::create_dir_all(addons.join("Jukebox")).unwrap();
    std::fs::write(addons.join("Jukebox").join("Track.mp3"), b"ID3!").unwrap();

    // The archive leg: a client track the chain does carry.
    let theme = read_chain_or_loose(
        &chain,
        Some(&addons),
        "Sound\\Music\\GlueScreenMusic\\wow_main_theme.mp3",
    )
    .expect("the glue theme lives in sound.MPQ");
    assert!(theme.len() > 1_000_000, "the theme is a ~3 MB stored MP3");

    // The loose leg: an addon's own track, named by the virtual path, spelt in its own case.
    assert_eq!(
        read_chain_or_loose(
            &chain,
            Some(&addons),
            "Interface\\AddOns\\Jukebox\\Track.mp3"
        )
        .as_deref(),
        Some(&b"ID3!"[..])
    );
    // In neither store: a plain miss, which every caller renders as silence.
    assert!(read_chain_or_loose(&chain, Some(&addons), "Sound\\Music\\nope.mp3").is_none());
    // No folder configured (a capture run, where `local_state` is hermetic): chain only.
    assert!(read_chain_or_loose(&chain, None, "Interface\\AddOns\\Jukebox\\Track.mp3").is_none());
    std::fs::remove_dir_all(&root).ok();
}
