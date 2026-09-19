# RustyFusion
RustyFusion is an open-source server emulator for Cartoon Network's MMO Fusionfall written in Rust inspired by the [OpenFusion project](https://github.com/OpenFusionProject) in which I am an active contributor. RustyFusion was initially an experiment for me to gain experience writing Rust but is now on course for eventual feature-completion. **Please note that, until then, RustyFusion is NOT ready for use as a production Fusionfall server!**

## RustyFusion vs. OpenFusion
- **Compatibility:** RustyFusion targets **one client: the FFOne / "Retrobution" build 104 client**, and speaks its network protocol exclusively. There is no protocol switch and no support for stock build 104 or "Academy" clients — see [Supported client](#supported-client). RustyFusion's SQL-based backends still use the OpenFusion database schema, and RustyFusion uses the same tabledata repository as OpenFusion for data sourcing. It is fully compatible with [ofapi](https://github.com/OpenFusionProject/ofapi) for secure login and account administration. It also supports the [OpenFusion monitor protocol](https://github.com/OpenFusionProject/ffmonitor) and is thus compatible with [OpenFusionMap](https://github.com/OpenFusionProject/OpenFusionMap-rs) and [computress](https://github.com/OpenFusionProject/computress-rs).
- **Performance:** RustyFusion is built on fully asynchronous I/O, including network and database operations. Where OpenFusion's main loop services clients serially with blocking DB access, RustyFusion can service packet handling and database queries across many clients concurrently on a **multi-threaded runtime**. In informal testing this translates to roughly a **5–10x improvement in concurrent request throughput** under load, as well as the ability to execute long-running DB operations (such as bulk player saves) without freezing the server.
- **Extensibility:** RustyFusion ships with a **Luau engine**, and all mob behavior is defined at a high level in Luau scripts with a growing API surface. Custom scripts can be added and assigned to mobs and NPCs through commands, and scripts can be **hot-reloaded at runtime**.
- **Safety:** Because RustyFusion is written in Rust as opposed to OpenFusion's choice of C++, it is, in theory, **much less** prone to memory safety issues, security vulnerabilities, and undefined behavior than OpenFusion's implementation of the game while maintaining the performance of compiled machine code.
- **Scalability:** Unlike OpenFusion, RustyFusion's login server and shard server can be compiled as **two separate binaries** that communicate to each other over the network, allowing for a more flexible server architecture with multiple shard servers. The login server can select the optimal shard for a player based on **geographic location** as well as shard population. A fallback mechanism is also in place to move players to another shard if the shard they are on fails gracefully for whatever reason.
- **Reliability:** RustyFusion comes after years of writing, refactoring, and evaluating OpenFusion code. There were a handful of cut corners and bad design decisions made in the development of OF that this project aims to avoid. Some already implemented examples include the increased usage of high-level types, a proper logging system, strict error-handling, and stricter packet validation ("anti-cheat"). These changes should lead to a cleaner codebase with less bugs.
- **Completeness:** OpenFusion is not technically complete in and of itself. There's a handful of features that remain unimplemented at the time of writing, such as channels, certain built-in commands, and NPC v. NPC combat. RustyFusion aims to close the gap on as many of these features as possible.

## What's Done and Left To Do (Roughly)
- [x] Core login server functionality
  - [x] Client connection
  - [x] Shard connection
  - [x] Auto account creation
  - [x] Character creation
  - [x] Character deletion
  - [x] Character selection
  - [x] Shard selection +
  - [x] Shard querying (channel + player info) +
- [x] Core shard server functionality
  - [x] Login server connection
  - [x] Client connection
  - [x] Channels +
  - [x] MOTD +
- [x] Config and tabledata frameworks
- [x] Core database functionality
  - [x] Framework +
  - [x] Account loading
  - [x] Player loading & saving
  - [x] Periodic shard auto-saving +
- [x] Optional Terminal UI *(bonus)*
- [x] Chunking
  - [x] Framework
  - [x] Entity tracking
  - [x] Instancing (infected zones + other private instances)
- [x] Travel
  - [x] S.C.A.M.P.E.R. (fast-travel)
  - [x] Monkey Skyway System (wyvern style)
  - [x] Sliders (bus style)
  - [x] Vehicles
  - [x] Warping through NPCs
- [x] Items
  - [x] Framework (equipping, stacking, deleting, etc)
  - [x] Vendors (buying, selling, buy-backs)
  - [x] Croc-Potting
  - [x] Trading
  - [x] C.R.A.T.E. opening
- [x] Social features
  - [x] Basic chat
  - [x] Buddies
    - [x] Framework
    - [x] Buddy chat [(thanks **lwcasgc**!)](https://github.com/yungcomputerchair/RustyFusion/pull/11)
    - [x] Buddy warping [(thanks **lwcasgc**!)](https://github.com/yungcomputerchair/RustyFusion/pull/12)
    - [x] Emails
    - [x] Blocking
  - [x] Groups
    - [x] Framework +
    - [x] Group chat
    - [x] Shared kills
    - [x] Group warping
- [x] Nanos*
  - [x] Swapping equipped nanos
  - [x] Summoning nanos
  - [x] Acquiring nanos
  - [x] Changing nano powers
  - [x] Stamina drain + regen
- [x] Combat
  - [x] Mobs
  - [x] Core combat loop & mob AI +
  - [x] Player respawning
  - [x] GM PvP
  - [x] Abilities and (de)buffs
    - [x] Framework
    - [x] All passive skills (including nano)
    - [x] All active skills (including nano)
    - [x] Gumballs & other usables
    - [x] E.G.G.s (the ones on the ground that buff you)
  - [x] Rockets and grenades
  - [x] Mob drops
- [x] Missions
  - [x] Starting tasks +
  - [x] Switching active mission
  - [x] Quest items
  - [x] Completing tasks +
  - [x] Mission rewards
  - [x] Escort tasks +
      - [x] Follow player
      - [x] Follow path
        - [x] Eduardo (Scary Monsters)
        - [x] Billy (Carnival Collection)
        - [x] Grim (Don't Fear the Reaper (Part 4 of 4))
- [x] Entity pathing
- [x] Infected Zones
  - [x] Movement elements
  - [x] Races
- [x] Guide changing
- [x] Admin features
  - [x] Built-in cheat commands +
  - [x] Custom command system
  - [x] Account (un)banning
  - [x] OpenFusion monitor protocol using [ffmonitor](https://github.com/OpenFusionProject/ffmonitor)
    - [x] `player` events
    - [x] `chat` events
    - [x] `bcast` events
    - [x] `email` events
    - [x] `namereq` events
  - [x] ofapi support
    - [x] OpenFusion DB version 6 compliance
    - [x] Login cookie support
- [x] Time machine
- [x] Fuse boss fight
- [x] Scripting API *(bonus)*
- [x] Banking +
  - [x] Main bank (200 slots)
  - [x] Membership banks +

### Supported client
RustyFusion speaks the network protocol of the **FFOne / "Retrobution" build 104
client** and nothing else. This is a deliberate choice: supporting several game
versions side by side means every feature has to be written twice and tested
twice, and the divergences are not cosmetic. Concretely, this client differs
from stock build 104 in that it:

- carries a 12-bit checksum of the packet body in the upper bits of the packet
  ID field, and starts from a different pre-handshake encryption key;
- uses expanded character creation palettes (36 skin, 54 hair, 10 eye colors);
- streams its nano book through `P_FE2CL_REP_NANO_BOOK_SUBSET` instead of
  relying on the fixed 37-nano prefix in the login packet;
- has a 200-slot bank plus four membership banks, and passes the banker NPC in
  the bank-open request;
- asks for the set of nearby NPC types up front (`P_CL2FE_REQ_PRESENT_NPC_TYPES`)
  so it can prefetch their assets;
- renders non-ASCII text, so chat, emails and the MOTD are not stripped to ASCII.

A stock build 104 client will not get past the handshake.

### Known Issues
- The AI script for mob pack followers currently errors out.

Items that are ***highlighted*** are in planning or WIP. Items marked with `+` are either new and not present in OpenFusion or enhanced from OpenFusion (bug fixes not included). Some items have dependencies in other categories, so the list won't get completed in order.

## Developing
The development commands follow FFOneClient. The pinned toolchain is Rust 1.98.1;
development builds use optimization level 1 with line-table debug information,
dependencies use level 3, and release builds use thin LTO with one codegen unit.
Windows MSVC builds use `rust-lld.exe`.

```sh
cargo dev          # build and run the combined login/shard server
cargo release      # build target/release/hybrid (hybrid.exe on Windows)
cargo check-all    # check every target
cargo test-all     # test every target
```

These aliases use SQLite by default. SQLite is bundled into the executable;
no database service or Docker is required. On first launch the server creates
`general.db_path` (default `database.db`) and initializes its tables from `sql/`.
An existing database is reused.

```sh
cargo dev
cargo release
# Runtime overrides go after Cargo's -- separator:
cargo dev -- --general.enable_tui=false
# Optional PostgreSQL build (requires a separately running PostgreSQL server):
cargo release --no-default-features --features postgres
```

`cargo release` builds without launching, just like FFOneClient. Separate `login`
and `shard` binaries remain available through the ordinary Cargo commands below.
See [the local OpenFusion migration audit](docs/openfusion-migration-audit.md)
for verified compatibility and the remaining migration blockers.

SQLite is the default database backend. Configure its file path in `config.toml`. If explicitly building with PostgreSQL, configure its connection parameters and create the database first; the optional development `docker-compose.yml` can provision that service.

Cargo (Rust's package manager) makes working with Rust projects extremely easy to setup. Just clone the repo (recursively, to grab critical tabledata), build, and run:
```
git clone --recurse-submodules https://github.com/gsemaj/RustyFusion
cd RustyFusion
cargo build
cargo run --bin login  # for login server
cargo run --bin shard  # for shard server
cargo run --bin hybrid # for combined login/shard server
```
To force the server to load the config file from a location other than `config.toml`, you can override the path in the command line with `--config=`. You can also override any number of individual config settings with `--section.key=value`.
```
cargo run --bin shard --config=some_other_config.toml
cargo run --bin shard --general.db_port=1234 --shard.num_channels=2
```
To select PostgreSQL instead of the default SQLite backend:
```
cargo run --bin hybrid --no-default-features --features postgres
```

## Contributing
If you have code you want to contribute, make sure you follow the general code style and run the following commands before you commit your code (CI/CD will catch you if you don't):
```
cargo fmt
cargo clippy
```

## Other Notes
### On Creative Liberties
Although RustyFusion tries to match the front-end behavior of OpenFusion closely, there are some notable, intentional differences that exist between the two. I only diverged on these because I think they lead to an overall improved experience.
- Mob behavior may not be identical to OpenFusion. The behaviors are written in Lua from the ground-up. Although care is put in to ensure the combat experience is as close as possible to OpenFusion, there may be small, subtle differences.
- Not all OpenFusion custom commands will be supported (particularly "gruntwork" commands) and brand new custom commands are added as good use cases come up.

### Disclaimer
RustyFusion is a **personal project that I work on in my free time**, so it is unlikely to progress at a constant pace.

Feel free to email or DM me if you have any questions about the project or are interested in contributing.
