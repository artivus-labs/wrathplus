use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
static SERIAL: AtomicU64 = AtomicU64::new(0);

fn scratch() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "server-data-{}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn each_tool_runs_offline_with_the_client_read_only() {
    let words = |argv: Vec<OsString>| {
        argv.into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    };
    let argv = words(
        run_args(
            Path::new("/games/WoW 3.3.5/Data"),
            Path::new("/state/managed-server-data"),
            Some((501, 20)),
            "map_extractor",
            &["-i", "/client", "-o", "/server-data"],
        )
        .unwrap(),
    );
    assert_eq!(argv[..2], ["run", "--rm"]);
    assert!(argv.windows(2).any(|w| w == ["--network", "none"]));
    assert!(argv.windows(2).any(|w| w == ["--user", "501:20"]));
    assert!(argv
        .iter()
        .any(|a| a == "type=bind,source=/games/WoW 3.3.5/Data,target=/client/Data,readonly"));
    assert!(argv
        .iter()
        .any(|a| a == "type=bind,source=/state/managed-server-data,target=/server-data"));
    assert!(argv
        .windows(2)
        .any(|w| w == ["--entrypoint", "/opt/acore/bin/map_extractor"]));
    // The image, then the tool's own arguments; no shell anywhere.
    let image = argv.iter().position(|a| a == command::IMAGE).unwrap();
    assert_eq!(argv[image + 1..], ["-i", "/client", "-o", "/server-data"]);
    assert!(!argv
        .iter()
        .any(|a| a == "sh" || a == "-c" || a.contains("/bin/sh")));
    for forbidden in [
        "--privileged",
        "-v",
        "--volume",
        "compose",
        "13724",
        "18085",
    ] {
        assert!(!argv.iter().any(|a| a == forbidden), "{forbidden}");
    }
    let comma = run_args(Path::new("/a,b/Data"), Path::new("/out"), None, "x", &[]);
    assert!(comma.unwrap_err().contains("comma"));
}

#[test]
fn parts_run_the_extractors_in_order() {
    let names = |part| {
        tools(part, Navmesh::All)
            .into_iter()
            .map(|(tool, _)| tool)
            .collect::<Vec<_>>()
    };
    assert_eq!(names(Part::Maps), ["map_extractor"]);
    assert_eq!(names(Part::Vmaps), ["vmap4_extractor", "vmap4_assembler"]);
    assert_eq!(
        tools(Part::Vmaps, Navmesh::All)[1].1,
        ["Buildings", "vmaps"]
    );
    let all = &tools(Part::Mmaps, Navmesh::All)[0].1;
    assert_eq!(all, &["--config", "/opt/acore/bin/mmaps-config.yaml"]);
    let tile = &tools(Part::Mmaps, Navmesh::Tile(0, 32, 48))[0].1;
    assert_eq!(tile[2..], ["0", "--tile", "32,48"]);
    assert_eq!(Navmesh::parse("571").unwrap(), Navmesh::Map(571));
    assert_eq!(Navmesh::Tile(0, 32, 48).describe(), "map 0, tile 32,48");
    assert_eq!(Navmesh::parse("0,32,48").unwrap(), Navmesh::Tile(0, 32, 48));
    assert!(Navmesh::parse("0,64,1").is_err() && Navmesh::parse("x").is_err());
}

#[test]
fn a_part_counts_only_when_its_files_are_there() {
    let out = scratch();
    // map_extractor exits 0 when it finds no locale; an empty folder is not success.
    assert!(produced(&out, Part::Maps).unwrap_err().contains(".dbc"));
    std::fs::create_dir_all(out.join("dbc")).unwrap();
    std::fs::write(out.join("dbc/Map.dbc"), "x").unwrap();
    assert!(produced(&out, Part::Maps).unwrap_err().contains("maps/"));
    std::fs::create_dir_all(out.join("maps")).unwrap();
    std::fs::write(out.join("maps/0004331.map"), "x").unwrap();
    produced(&out, Part::Maps).unwrap();
    assert!(produced(&out, Part::Vmaps).is_err());
    std::fs::create_dir_all(out.join("vmaps")).unwrap();
    std::fs::write(out.join("vmaps/000.vmtree"), "x").unwrap();
    produced(&out, Part::Vmaps).unwrap();
    assert!(!done(&out, Part::Maps));
    std::fs::write(out.join(Part::Maps.marker()), "done").unwrap();
    assert!(done(&out, Part::Maps) && !done(&out, Part::Vmaps));
    // Intermediate output is removed; anything that is not a folder is left for the player.
    std::fs::create_dir_all(out.join("Buildings/dir_bin")).unwrap();
    remove_own(&out, "Buildings").unwrap();
    assert!(!out.join("Buildings").exists());
    std::fs::write(out.join("Buildings"), "mine").unwrap();
    assert!(remove_own(&out, "Buildings").is_err());
    let _ = std::fs::remove_dir_all(out);
}

#[test]
fn client_data_is_the_folder_with_common_mpq() {
    let root = scratch();
    assert!(client_data(Path::new(""))
        .unwrap_err()
        .contains("Client page"));
    std::fs::create_dir_all(root.join("WoW/Data")).unwrap();
    assert!(client_data(&root.join("WoW"))
        .unwrap_err()
        .contains("common.MPQ"));
    std::fs::write(root.join("WoW/Data/common.MPQ"), "x").unwrap();
    let data = client_data(&root.join("WoW/Data")).unwrap();
    assert_eq!(client_data(&root.join("WoW")).unwrap(), data);
    assert!(data.ends_with("WoW/Data"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn progress_reads_the_tools_own_counts() {
    let maps = "Extracting maps...\nConvert map files\nExtract Azeroth (1/155)                  \nProcessing........................42%\rExtract Kalimdor (2/155)   \n";
    assert_eq!(progress(maps), Some((2, 155)));
    let mmaps = "[Map 000] We have 2134 tiles.\n7% [Map 0000] Building tile [31,47]\n[Map 000] [31,47]: Writing to file...\n";
    assert_eq!(progress(mmaps), Some((7, 100)));
    assert_eq!(
        progress("Extract V4.00 2012_02. Beginning work ....\n"),
        None
    );
    assert_eq!(progress("Processing Map 0\n[####]\n"), None);
}
