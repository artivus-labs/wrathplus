# Detour (vendored)

The navigation-mesh query half of [Recast & Detour](https://github.com/recastnavigation/recastnavigation)
(zlib licence, `License.txt`), copied unchanged from AzerothCore's `deps/recastnavigation/Detour`
at AzerothCore commit `93333d6a9eb00fdf0179adde30da3c17618207d6` (the Playerbot fork the managed server builds).

It is AzerothCore's copy rather than upstream's because the server's mmaps are raw Detour tiles:
AzerothCore defines `DT_POLYREF64` in `DetourNavMesh.h` with its own salt/tile/poly bit split
(12/21/31; `deps/recastnavigation/recastnavigation.diff` in AzerothCore), and the client should run
the same query code the server runs. `DetourNavMeshBuilder` is left out (the client only reads
meshes). `crates/wrathplus-nav` compiles these files with a small C shim.
