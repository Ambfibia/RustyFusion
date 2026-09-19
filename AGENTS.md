# RustyFusion Agent Instructions

RustyFusion is the primary game server for FFOneClient. All new server behavior,
network integration and migration fixes belong here. `../OpenFusion` is the
legacy server retained for inspecting reference code, accepted local extensions,
behavior and persistence contracts.

- Support one FFOne / Retrobution 0104 protocol. Do not introduce protocol
  variants or compatibility switches unless explicitly requested by the owner.
- Compare migration behavior with the local OpenFusion checkout, not just its
  upstream project or the client's packet declarations.
- Preserve accepted native/server identifiers, Nano tuning identities, NPC
  mappings, content and player data. Primary-server status does not imply that
  migration is finished; see `docs/openfusion-migration-audit.md`.
- Nano acquisition and player progression are separate contracts. Some Nano
  acquisition paths advance a level; others award a Nano through a quest or
  item subject to level requirements without advancing progression. Preserve
  each path's prerequisites, FM/item costs and level effect. Never infer the
  target player level from the Nano ID or globally disable Nano level-ups.
  The local OpenFusion `Missions::isGrowthNanoMission` distinction and
  `AvatarGrowth` links are reference evidence for progression quests.
- FusionForge owns legacy extraction, conversion and reproducible content
  publication. Runtime integration, native server loaders and their tests belong
  here. Do not make the server require FusionForge or legacy builds at runtime.
- `cargo dev` runs the hybrid login/shard server; `cargo release` builds it.
  `cargo check-all` and `cargo test-all` cover all targets. SQLite is the
  default backend; pass `--no-default-features --features postgres` for PostgreSQL.
- Use isolated databases and distinct local ports for mutating migration tests.
  Keep disposable test data and reports below ignored `target/`.
