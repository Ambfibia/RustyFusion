# Migration from the local OpenFusion server

Audit date: 2026-09-18. Scope: the working checkouts of RustyFusion, FFOneClient,
and the sibling OpenFusion, including OpenFusion's deployed `bin/tdata` and a
read-only SQLite backup of `bin/database.db`. This is not a comparison with
upstream OpenFusion or a claim of full gameplay equivalence.

**Decision: suitable for isolated integration testing, not yet a replacement
for the current OpenFusion deployment.** Keep the single FFOne protocol;
the missing work is local behavior/content compatibility, not protocol selection.
The owner has designated RustyFusion as the primary server for ongoing work;
OpenFusion is now the legacy reference. The blockers below are migration work
for RustyFusion, not a recommendation to continue feature development in OpenFusion.

## Verified and corrected

- The initial key, checksum framing, login key derivation, FE/E key transition,
  character creation, shard selection, 2700-byte world-entry packet and
  76-byte Nano book pages work with the real `ffone-net` transport.
- The server was sending the persistent character UID in Nano book `PCUID`.
  OpenFusion's `src/PlayerManager.cpp::sendNanoBook` and FFOne's Nano runtime use
  the live shard player ID. Corrected RustyFusion accordingly.
- NPC presence pages contained 1021 entries. OpenFusion and FFOne cap this
  packet at 1020. The initial smoke failed with `CountTooLarge`; corrected
  RustyFusion's chunk capacity and repeated the smoke successfully.
- Of 510 shared client/server packet identifiers, one differed:
  `P_FE2CL_REP_REQUEST_MAKE_BUDDY_SUCC` was `0x7fffffff`, now `0x83000065`.
  This fixes the declaration; it does not claim a complete buddy-flow test.
- Main bank size 200 plus four extra banks is represented in both servers.
  Flat inventory storage is equipment 9 + inventory 50 + five banks of 200.
- The current SQLite database and RustyFusion's initialized database have the
  same table names and column name/type/not-null/primary-key descriptions;
  both report `DatabaseVersion=6`, `ProtocolVersion=104`. This does not prove
  every existing character survives a load/save cycle.

The generated legacy mirror in FFOne is not always the active network codec:
for example the active `PcLoadData0104` is 2688 bytes and `PcEnterSuccess` is
2700 bytes. Comparing only the generated 2552/2564-byte mirror would incorrectly
report a world-entry incompatibility. The active vendor table codec also uses
20 items (480 bytes), matching RustyFusion.

## Blocking differences from our OpenFusion

### 1. Current data does not load unchanged

Direct startup with `OpenFusion/bin/tdata` failed because RustyFusion requires
`worldnames.json`, which the deployed OpenFusion dataset does not contain.
An isolated copy supplemented only with RustyFusion's `worldnames.json` then
failed while reading NPC positions:

```text
Malformed NPC data entry: invalid type: floating point `54727.996826171875`, expected i32
```

`src/tabledata.rs::load_npcs` requires integer coordinates/angles. The current
OpenFusion input contains floating point coordinates. Define and test conversion
semantics against OpenFusion before accepting this dataset; do not replace our
placements with RustyFusion's different dataset to hide the failure.

RustyFusion's JSON loader also still has `TODO patching`; OpenFusion has a patch
application path. Patch behavior must be accounted for when preparing the
effective server dataset.

### 2. Bundled content is different

Counts below are source rows, not supported-feature counts:

| Dataset | Current OpenFusion `bin/tdata` | RustyFusion `tabledata` |
| --- | ---: | ---: |
| Nano data rows | 71 | 48 |
| Nano tuning rows | 291 | 109 |
| NPC placements | 3013 | 2807 |
| Mob placements | 6862 | 9032 |
| Mob groups | 385 | 784 |
| NPC path entries | 7 | 104 |

OpenFusion Nano IDs missing from RustyFusion's source rows:
`41–46, 48–51, 53–70`. Shared IDs `1–38` also have differing row values.
Do not interpret the extra rows on either side as automatically approved content.
The `drops.json` files match byte-for-byte. `eggs.json` differs in formatting but
its parsed JSON values match. XDT, NPCs, mobs and paths differ semantically.

### 3. Request handlers are missing

The current OpenFusion source registers 133 distinct shard request IDs.
RustyFusion has no dispatch arm for these 12:

| Family | Missing requests |
| --- | --- |
| GM street stall | `P_CL2FE_PC_STREETSTALL_REQ_READY`, `REGIST_ITEM`, `UNREGIST_ITEM`, `SALE_START`, `ITEM_LIST`, `ITEM_BUY` (same prefix) |
| Environment damage | `P_CL2FE_DOT_DAMAGE_ONOFF` |
| Recall | `P_CL2FE_REQ_REGIST_RXCOM`, `P_CL2FE_REQ_WARP_USE_RECALL` |
| Trade cancellation | `P_CL2FE_REQ_PC_TRADE_OFFER_ABORT` |
| GM travel | `P_CL2FE_REQ_PC_WARP_TO_PC` |
| GM Nano skill | `P_CL2FE_REQ_PC_GIVE_NANO_SKILL` |

These are present in our server, including `src/GMStore.cpp`; they are not just
unused protocol declarations. RustyFusion's street-stall cancel handler alone
does not reproduce the store. OpenFusion's DOT request activates/removes the
infection buff and its damage ticks.

The login server also lacks `P_CL2LS_REQ_CHANGE_CHAR_NAME`, handled by our
OpenFusion's `src/servers/CNLoginServer.cpp` and sent by FFOne.

### 4. Nano behavior is not equivalent

- OpenFusion treats `m_iTune` as **tuning IDs**, validates the requested tuning
  against the Nano's list, then maps through `m_iSkillID`. RustyFusion stores
  `m_iTune` in `NanoStats.skills` and compares the resolved **skill ID** to that
  list in `Player::tune_nano`. This fails when tuning IDs and skill IDs differ,
  as allowed by our content contract. The deployed XDT contains 58 such Nano/tune
  associations; e.g. Nano 38 maps tuning 201 to skill 13.
- Preserve the owner's distinction: some Nano acquisition paths advance a
  level, while other quest/item rewards are gated by level without advancing
  progression. Do not remove all Nano level-ups. Our OpenFusion explicitly
  classifies growth missions with `Missions.cpp::isGrowthNanoMission`, matching
  the mission to `AvatarGrowth.m_iNanoQuestTaskID`; those grants spend growth FM
  and advance one level. Other Nano mission rewards grant ownership without
  that growth effect. GM grants also preserve level. RustyFusion currently uses
  `max(current_level, nano_id)` in **both** `gm_pc_give_nano` and Nano mission
  completion, and the latter always subtracts the growth FM cost. Migrate the
  acquisition-path distinction and prerequisites, not a global level-up policy.
- Paid tuning uses the current AvatarGrowth level's cost in our OpenFusion,
  whereas RustyFusion uses the tuning row's cost. Free first tuning is present
  in both, but that alone does not establish tuning parity.
- RustyFusion's `pc_regen` still uses `todo!()` for `HereByPhoenix` and
  `HereByPhoenixGroup`. Our OpenFusion handles both revive modes. A matching
  packet ID does not make those paths usable; they currently panic.

### 5. Deployment defaults differ

The current OpenFusion config accepts password login and defaults new accounts
to level 1. RustyFusion's config disables password login for release builds
(debug builds override this) and defaults later accounts to level 99. OpenFusion
saves every 240 seconds; RustyFusion is configured for five minutes. RustyFusion
now uses SQLite by default, matching the local database deployment model.
PostgreSQL is opt-in with `--no-default-features --features postgres`.
Choose these settings deliberately before a switch. Do not assume a passing
debug password login also proves release login with the default config.

## Validation and remaining acceptance

Passed:

- `cargo test -p ffone-protocol -p ffone-net --locked`: 107 protocol and 30
  transport tests.
- `cargo check-all --locked --no-default-features --features sqlite`.
- `cargo test-all --locked --no-default-features --features sqlite`: 33 tests,
  including SQLite save/reload, emails and racing.
- `cargo release --locked`: PostgreSQL hybrid release build at the time of the
  initial audit, before the default was changed to SQLite.
- Real isolated SQLite hybrid + FFOne `server_migration_smoke`: password login,
  name reservation, appearance creation at expanded palette limits, tutorial
  save, character/shard selection, world entry, Nano ownership, initial entity
  decoding, NPC presence and acknowledged clean exit.

The smoke lives in FFOneClient at
`crates/ffone-net/examples/server_migration_smoke.rs`. It requires
`FFONE_ISOLATED_TEST_DATABASE=1`, `FFONE_LOGIN_ADDRESS`, `FFONE_USERNAME`,
`FFONE_PASSWORD` and runs with
`cargo run -p ffone-net --example server_migration_smoke --locked`.
It creates a character if the test account is empty; never use it on the live DB.
Runtime logs and copies are under ignored `target/` directories.

September 18 tutorial-entry regression: FFOne runs the unfinished tutorial on the
shared shard. Character selection now accepts an account-owned character with
`tutorial_flag=false`, matching the local OpenFusion selection handler, without
changing the flag or granting completion rewards. The previous completion-only
smoke did not exercise this route. Set `FFONE_SMOKE_TUTORIAL_ENTRY=1` on a fresh
isolated test account to verify creation, selection, scripted tutorial entry,
loading and clean exit. Restart the test server and repeat on the same database
to verify that the persisted tutorial flag remains false, then restart and run
the ordinary smoke to cover completion and normal world entry too.

The full-client follow-up found a second handoff incompatibility: the login
server closed the client immediately after SHARD_SELECT_SUCC. FFOne retains that
authenticated connection during gameplay; its reader treated EOF as a session
failure after WorldReady, leaving the tutorial loading overlay visible. The
server now keeps the login socket alive, matching the local OpenFusion behavior.
The client also clears its loader on disconnect after WorldReady.

The earlier release-only reconnect probe returned AlreadyLoggedIn after this
forced handoff closure; debug builds bypass duplicate-session checks. Full-client
verification must include the retained socket and normal shutdown instead of
testing only ShardSession in isolation. The FFOne
`FFONE_TUTORIAL_NETWORK_PROBE_OUTPUT` fixture covers the real login worker,
cutscene, WorldReady and twelve seconds of admitted gameplay/keepalives. It uses
an explicitly isolated test database and credentials, and stores reports under
FFOneClient's ignored `target/performance/tutorial-*` tree.

The installed release pair subsequently passed that full-client fixture and a
second login/shard-entry smoke on the same server without restarting it. The
server logged normal player departure and login-session cleanup. Packaged-run
evidence is under FFOneClient `target/performance/tutorial-package-*`.

Still required: effective content parity, full existing-character load/save
comparison on a copied DB, the missing handlers, Nano tuning/progression parity,
two-player trade/buddy/group tests, combat/escort/race/transport acceptance and
release authentication against the intended backend. Performance has not been
benchmarked. No production endpoint, database, or deployed OpenFusion content
was switched by this audit.
