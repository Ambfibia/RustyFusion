# Native instance route repairs, 16 October 2026

The requested completion of the previously unreachable maps is a production
server/content migration. It keeps all 151 existing nonzero map identities and
51 geometry cells. The ordinary XDT loader, portal packet handler and client
portal window consume these routes; acceptance does not create portal actors or
turn off prerequisites. `tools/repair_instance_routes.py` applies the migration
idempotently and `--check` detects conflicting/missing production edits.

This work contains both corrections supported by the existing table links and
explicitly authored native extensions. **The restored placements and the three
new destination choices are not claimed as recovered Retrobution placements or
original gameplay parity.** The accepted request to finish the routes is the
scope for authoring these missing pieces. No new NPC type, mesh identity or
player progression level is invented.

## Corrections supported by existing links

The local OpenFusion `src/TableData.cpp` loads `m_iToMapNum` and destination XYZ
without repairing the data. Its current `tdata/NPCs.json` has the same missing
actors and map-150 mismatch as the RustyFusion baseline. Copying that behavior
cannot make the missing geometry routes reachable.

| Identity | Correction | Evidence already in production tables |
| --- | --- | --- |
| 142, 143, 144 | Declared cell `(7,6)` becomes `(7,5)`; XYZ stays exact. | Every incoming warp, 305/307/309, lands in `(7,5)`. All three shared the same conflicting cell declaration. Their paired returns 306/308/310 reach their distinct Pokey Oaks overworld entrances. Both cells are published, so this changes the selected scene and retains the geometry guard. |
| 149 | Warp 317 targets 149 instead of undefined 150. Tasks 5236/5237 require 149 instead of undefined 150. | The original destination `(630715,808964,4003)` matches map149's declared `(12,15)` and its existing exit3343 at `(630500,809000,4000)`. The exit's warp318 returns next to original entry3339. The two Nano843 tasks were the only task instance references to150. |
| 151 | Declared cell `(12,15)` becomes `(15,9)`. Existing3345 placement IDs2874/2903 move from undefined150 to151; XYZ stays exact. | Incoming warp319 lands at `(793995,492381,0)` in `(15,9)`, adjacent to exit3345 at `(793994,492381,30)`. Both3345 placements lie in this cell. Nano844 already requires151 and those task IDs stay unchanged. |

Map150 is not added or silently aliased to a valid map. Its inconsistent inbound
warp, mission gates and placements are each reconciled to their corresponding
existing identities. Unrelated malformed original alternatives282/286 remain
rejected by the existing coverage guard; map135 still has valid warp284.

## Explicit authored placement repairs

Placement keys40000–40020 are reserved for this migration. Missing portal types
are instantiated at their paired warp's exact original endpoint. These numbers
are source-derived anchors, not evidence that the original client placed an
actor at that point. The actor exists in the production server tables for normal
clients, outside the validation runner.

| Map | Entry type and anchor | Return type and anchor |
| --- | --- | --- |
| 2 | 679 inside75 at warp4's endpoint; normal route to75 remains required. | 680 inside2 at warp3's endpoint, preserving warp4's original return to75. |
| 5 | 670 in overworld at warp10's endpoint. | 671 inside5 at warp9's endpoint. |
| 8 | 677 in overworld at warp17's endpoint. | 678 inside8 at warp16's endpoint. |
| 18 | 1048 in overworld at warp37's endpoint. | 1049 inside18 at warp36's endpoint. |
| 130 | 3123 in overworld at warp277's endpoint. | 3128 inside130 at warp272's endpoint. The pre-existing3128 actor is preserved. |
| 142 | 693 in overworld at warp306's endpoint. | 1587 inside142 at warp305's endpoint. |
| 143 | 1588 in overworld at warp308's endpoint. | 1593 inside143 at warp307's endpoint. |
| 144 | 1597 in overworld at warp310's endpoint. | 1598 inside144 at warp309's endpoint. |

All original task, level, item, item-consumption, cost and group conditions on
these warps remain unchanged. In particular map2 keeps task5075, map130 keeps
task2703, maps142/143 keep tasks5200/5208, and map144 consumes item36/type7.

## Explicit additional destination choices

Maps4/136/152 originally had neither an incoming warp nor a mission-owned entry.
Their identities remain separate from their geometry-sharing neighbors. Existing
normal portal actors now offer an additional destination through the ordinary
portal UI. These are native extensions, not repaired original warp identities.

| New warp | Map | Existing portal and geometry anchor | Preserved prerequisites |
| --- | --- | --- | --- |
| 321 | 4 | Type681, copy warp5's map3 XYZ. Exit683 uses original placement2832's exact XYZ inside4. | Warp5's level2 and consumed item1/type7, group and cost rules. |
| 322 | 136 | Type3283, copy warp291's map137 XYZ. Exit3289 sits at that anchor and uses original warp292. | Warp291's level4, group and cost rules. |
| 323 | 152 | Type3344, copy warp319's corrected map151 XYZ. Copies of original exit3345 placements2874/2903 provide return via320. | Warp319's task5238, group and cost rules. |

## Nano mission842 continuation

Maps146–148 retain the original mission842 task graph. Leaving145/146/147 uses
the original instance-leave failure transitions to5227/5233/5231 rather than
seeding incompatible stages together. The map147 stage5229 requires an actual
kill of3337 and a living escort3338. Their absent production spawns are authored
beside original portal3330 `(628729,788033,2314)`:

- `NPCs.json` placement40020: escort3338 at `(628329,788033,2314)`, map147.
- `mobs.json` spawn40020: hostile3337 at `(628329,787633,2314)`, map147.

These retain original XDT NPC types/stats (3337 has4000 HP,3338 has50000 HP),
mission targets, failure routes, Nano progression and item costs. The hostile is
retained by the isolated noncombat acceptance fixture because killing it is an
actual mission requirement. No substitute NPC or direct task-state mutation is
part of the route.

## Validation boundary

The focused Rust tests load the production server catalog, require a placed
portal graph reaching every one of 151 identities, verify explicit return routes
for the 13 repaired entries, reject unexpected destinations outside declared
cells, and compare new choices' access/consumption rules with their donors.
The migration script's check verifies idempotence and preservation of colliding
IDs. This static/server-loader validation does not establish runtime scene and
collider admission or visible rendering. The independent final Unity acceptance
of all 151 maps passed on 16 October 2026, verifying a selected route for every
map identity and the real mission 842 sequence. It checked 140 explicit NPC exits and 11 saved pre-warp
overworld fallbacks using ordinary accounts with permission level 99; character
levels follow each route's prerequisites. Controlled positioning before portals
does not establish continuous walking access through every room or corridor,
and selected-route coverage does not cover every alternative warp.

The [final Unity acceptance receipt](../../FFOneUnity/Tools/LiveGameplay/acceptance-20261016-instance-routes-recall.json)
records 151 server and 151 native-player routes across 51 cells, zero scene-only
cases, the complete 210-test server library suite, 284 Unity Editor tests, and
the separate final live Recall run (220 network / 196 player checks). Its
`fullMatrixRerunOnFinalBinary=true` requires the full matrix and Recall to share
the delivered DLL and built Addressables catalog, with current source and
executable fingerprints and rendered captures. All owned test servers stopped;
ordinary player databases were not used.
