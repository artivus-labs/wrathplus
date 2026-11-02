//! Compiles the vendored Detour (`third_party/detour`, AzerothCore's copy, zlib) and the C shim.
use std::path::PathBuf;

fn main() {
    let detour = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../third_party/detour");
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
    println!("cargo:rerun-if-changed=../../third_party/detour");
}
