<div align="center">
  <h1>WrathPlus</h1>
  <p><b>A World of Warcraft 3.3.5a client, written from scratch in Rust and <a href="https://bevy.org">Bevy</a></b></p>
</div>

WrathPlus is a new client for Wrath of the Lich King 3.3.5a. It runs the stock 3.3.5 interface from
your own game files and plays on AzerothCore realms, including one the launcher can run for you on
your own computer, complete with computer-controlled party members.

Everything is written from scratch: the file formats, the network protocol and the interface
engine. There's no original client code and no game content in this repository; you bring your
own 3.3.5a (build 12340) game files.

## Getting started

You need [Rust](https://rustup.rs) and a copy of the 3.3.5a game files. Then:

```sh
git clone https://github.com/artivus-labs/wrathplus.git
cd wrathplus
make manager
```

That builds and opens the **WrathPlus launcher**. From there:

1. **Client** — point it at your game's `Data` folder, then press **Build client**.
2. **Server** — want to play offline? The launcher can build and run a private AzerothCore realm
   with Playerbots, using [Docker Desktop](https://www.docker.com/products/docker-desktop/).
   Prefer an existing realm? Just enter its address instead.
3. **Play** — start the server and the client from the front page, and log in.

Platform specifics (macOS, Linux, Windows and WSL), command-line setup and the details behind each
step are in [Client setup](docs/CLIENT-SETUP.md).

## What the launcher does

- **Client setup:** checks your game files, builds the client and launches it.
- **Your own realm:** a private AzerothCore server in Docker, started and stopped with one button.
  Your characters and progress are kept when it stops.
- **Server modules:** one-click cards for the popular extras: Playerbots, the MultiBot bridge,
  LLM Chatter, guild houses and an auction house bot. Any other AzerothCore module can be added
  from GitHub or a local folder.
- **Addons:** add client addons from GitHub or a folder, and switch them on per character as usual.

## Status

WrathPlus is playable but young. Questing, combat, professions, groups with bots, mail, trade and
most of the stock interface work; rough edges remain. Known bugs and planned work are on the
[issue tracker](https://github.com/artivus-labs/wrathplus/issues) and the
[project board](https://github.com/users/artivus-labs/projects/4). Bug reports are very welcome.

There are two experience presets: **Classic**, the default, which matches the stock 3.3.5 client,
and **Plus**, an optional set of modern conveniences.

## What's inside

- **Formats:** readers for the whole 3.3.5 asset stack: MPQ archives, BLP textures, DBC tables,
  ADT/WDT/WDL terrain, M2 models and WMO buildings.
- **World:** streamed terrain, buildings, foliage, water, sky, weather and lighting.
- **Networking:** the build-12340 login and world protocol.
- **Interface:** a FrameXML and Lua engine that runs the stock interface, and addons, from your
  own game files.
- **Audio:** music, ambience and effects.

[`docs/MAP.md`](docs/MAP.md) lists every crate and subsystem. Reference notes are on the
[wiki](https://github.com/artivus-labs/wrathplus/wiki), and
[`docs/CONTRIBUTING.md`](docs/CONTRIBUTING.md) covers building, testing and sending changes.

---

WrathPlus grew out of [benilla](https://github.com/samwhosung/benilla), a from-scratch 1.12 client,
and owes a great deal to its author and contributors. Early inspiration and file-format guidance
came from the [wowemulation-dev](https://github.com/wowemulation-dev) community, and
[warcraft-rs](https://github.com/wowemulation-dev/warcraft-rs) in particular.

This is an independent fan project, not affiliated with or endorsed by Blizzard Entertainment. It
ships **no Blizzard content**: no art, models, sounds, maps, MPQ contents or FrameXML. You provide
your own legally obtained 3.3.5a client, and the stock interface runs from its files at runtime.
The few files under `crates/wrathplus-app/assets/ui/` are our own: adapters over stock files, and
the settings windows and script error log the client draws itself.

World of Warcraft is a trademark of Blizzard Entertainment, Inc. Our own code is licensed under
[MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option. The two vendored components
under `third_party/`, the kira audio engine and a patched Lua 5.1, keep their own upstream
licenses, alongside each; their `BENILLA.md` files record what was changed and why.
