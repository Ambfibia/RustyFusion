"""Apply the documented native instance-route repairs, preserving original IDs.

This is an idempotent production table migration, not acceptance-fixture setup.
The server and client publication consume its resulting ordinary XDT/NPC tables.
See docs/instance-route-repairs.md for the source versus authored-extension split.
"""
import argparse
import copy
import json
from pathlib import Path


def repair(root):
    root = Path(root)
    paths = {name: root / name for name in ('xdt.json', 'NPCs.json', 'mobs.json')}
    data = {name: json.loads(path.read_text(encoding='utf-8')) for name, path in paths.items()}
    xdt, npcs, mobs = data['xdt.json'], data['NPCs.json']['NPCs'], data['mobs.json']['mobs']
    table = xdt['m_pInstanceTable']
    maps = {r['m_iInstanceNameID']: r for r in table['m_pInstanceData']}
    warps = {r['m_iWarpNumber']: r for r in table['m_pWarpData']}

    def change(row, field, before, after):
        if row[field] not in (before, after):
            raise ValueError(f'Conflicting repair {field}: {row[field]}')
        row[field] = after

    for map_id in (142, 143, 144):
        change(maps[map_id], 'm_iZoneY', 6, 5)
    change(maps[151], 'm_iZoneX', 12, 15)
    change(maps[151], 'm_iZoneY', 15, 9)
    change(warps[317], 'm_iToMapNum', 150, 149)
    for task in xdt['m_pMissionTable']['m_pMissionData']:
        if task['m_iHTaskID'] in (5236, 5237):
            change(task, 'm_iRequireInstanceID', 150, 149)
    for placement in ('2874', '2903'):
        change(npcs[placement], 'iMapNum', 150, 151)

    def add_warp(warp_id, donor, map_id, name):
        row = copy.deepcopy(warps[donor])
        row.update(m_iWarpNumber=warp_id, m_iToMapNum=map_id)
        if warp_id in warps:
            if warps[warp_id] != row:
                raise ValueError(f'Warp ID {warp_id} already has different content')
        else:
            table['m_pWarpData'].append(row)
            warps[warp_id] = row
        names = table['m_pWarpNameData']
        if len(names) == warp_id:
            names.append({'m_pstrNameString': name})
        elif len(names) <= warp_id or names[warp_id] != {'m_pstrNameString': name}:
            raise ValueError(f'Conflicting warp name ID {warp_id}')

    # Previously unreachable map identities remain distinct. Ordinary portals
    # offer explicit additional destinations and retain their donor's gates.
    add_warp(321, 5, 4, 'Bunker - additional instance')
    add_warp(322, 291, 136, 'Fusion Lair - additional instance')
    add_warp(323, 319, 152, "Fuse's Back Door - additional instance")

    def add_placement(rows, key, npc_type, map_id, x, y, z, angle=0):
        row = dict(iAngle=angle, iMapNum=map_id, iNPCType=npc_type, iX=x, iY=y, iZ=z)
        key = str(key)
        if key in rows and rows[key] != row:
            raise ValueError(f'Placement ID {key} already has different content')
        rows[key] = row

    def at_warp(key, npc_type, map_id, warp_id):
        w = warps[warp_id]
        add_placement(npcs, key, npc_type, map_id, w['m_iToX'], w['m_iToY'], w['m_iToZ'])

    # Paired original endpoints provide exact positions. Their placement is an
    # explicit native repair; no original spawn coordinates are claimed.
    for row in (
        (40000, 679, 75, 4), (40001, 680, 2, 3),
        (40002, 670, 0, 10), (40003, 671, 5, 9),
        (40004, 677, 0, 17), (40005, 678, 8, 16),
        (40006, 1048, 0, 37), (40007, 1049, 18, 36),
        (40008, 3123, 0, 277), (40009, 3128, 130, 272),
        (40010, 693, 0, 306), (40011, 1587, 142, 305),
        (40012, 1588, 0, 308), (40013, 1593, 143, 307),
        (40014, 1597, 0, 310), (40015, 1598, 144, 309),
        (40017, 3289, 136, 291),
    ):
        at_warp(*row)
    # Existing Bunker/Fuse exit actors retain original NPC identities and XYZ.
    for key, donor, map_id in ((40016, '2832', 4), (40018, '2874', 152), (40019, '2903', 152)):
        a = npcs[donor]
        add_placement(npcs, key, a['iNPCType'], map_id, a['iX'], a['iY'], a['iZ'], a['iAngle'])

    # The original Nano-842 chain expects these types but omits their spawns.
    # Anchor the authored continuation beside the original map-147 portal3330.
    add_placement(npcs, 40020, 3338, 147, 628329, 788033, 2314)
    add_placement(mobs, 40020, 3337, 147, 628329, 787633, 2314)
    return {name: json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + '\n'
            for name, value in data.items()}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tables', type=Path, default=Path(__file__).resolve().parents[1] / 'tabledata')
    parser.add_argument('--check', action='store_true', help='Verify production tables already include the repairs.')
    args = parser.parse_args()
    changed = []
    for name, expected in repair(args.tables).items():
        path = args.tables / name
        if path.read_text(encoding='utf-8') != expected:
            changed.append(name)
            if not args.check:
                path.write_text(expected, encoding='utf-8', newline='\n')
    if args.check and changed:
        raise SystemExit('Missing production repairs: ' + ', '.join(changed))
    print(('Verified' if args.check else 'Applied') + ' native instance routes: ' + (', '.join(changed) or 'already current'))
