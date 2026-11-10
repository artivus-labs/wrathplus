//! Compiles the vendored Detour (`third_party/detour`, AzerothCore's copy, zlib) and the C shim.
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let detour = root.join("third_party/detour");
    // The realm's route module (`mod-agent-nav`), from its own sources, for this crate's tests:
    // nothing in the client references it, so it is not linked into the game. Before Detour, which
    // it calls: a linker that reads archives in order must meet the caller first.
    let module = root.join("packaging/server/modules/mod-agent-nav/src");
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .include(detour.join("Include"))
        .include(&module)
        .define("DT_POLYREF64", "1")
        // As Detour below is built: its filter class has no type info to inherit.
        .flag_if_supported("-fno-rtti")
        .warnings(true)
        .file("csrc/realm_module_shim.cpp")
        .compile("wrathplus_realm_nav_module");
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .include(detour.join("Include"))
        .define("DT_POLYREF64", "1")
        .flag_if_supported("-fno-exceptions")
        .flag_if_supported("-fno-rtti")
        .warnings(false);
    for file in [
        "DetourAlloc.cpp",
        "DetourAssert.cpp",
        "DetourCommon.cpp",
        "DetourNavMesh.cpp",
        "DetourNavMeshQuery.cpp",
        "DetourNode.cpp",
    ] {
        build.file(detour.join("Source").join(file));
    }
    build.file("csrc/nav_shim.cpp");
    build.compile("wrathplus_detour");
    println!("cargo:rerun-if-changed=csrc/nav_shim.cpp");
    println!("cargo:rerun-if-changed=csrc/realm_module_shim.cpp");
    println!("cargo:rerun-if-changed=../../third_party/detour");
    println!("cargo:rerun-if-changed=../../packaging/server/modules/mod-agent-nav/src");
}
