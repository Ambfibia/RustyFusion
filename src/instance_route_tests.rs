//! Production-table reachability and prerequisite preservation for native repairs.
use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::tabledata::tdata_init;

fn source(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tabledata").join(name);
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn production_instance_catalog_has_placed_routes_for_all_151_identities_and_repaired_returns() {
    let tables = tdata_init().unwrap();
    let xdt = source("xdt.json");
    let npcs = source("NPCs.json");
    let instance = &xdt["m_pInstanceTable"];
    let maps: HashSet<u32> = instance["m_pInstanceData"].as_array().unwrap().iter()
        .map(|v| v["m_iInstanceNameID"].as_u64().unwrap() as u32).filter(|m| *m > 0).collect();
    assert_eq!(maps.len(), 151, "Map identities must not be merged to manufacture coverage");
    let mut links: HashMap<u32, HashSet<u32>> = HashMap::new();
    for row in instance["m_pWarpData"].as_array().unwrap() {
        let id = row["m_iWarpNumber"].as_i64().unwrap() as i32;
        if id <= 0 { continue; }
        let warp = tables.get_warp_data(id).unwrap();
        if warp.map_num != 0 {
            let cell = tables.get_map_data(warp.map_num).unwrap().map_square;
            if (warp.pos.x.div_euclid(51200), warp.pos.y.div_euclid(51200)) != cell {
                // The two unrelated, still-invalid original alternatives for
                // map135 do not become accepted routes merely by existing.
                assert!([282, 286].contains(&id), "Unexpected bad destination warp {id}");
                continue;
            }
        }
        for npc in npcs["NPCs"].as_object().unwrap().values() {
            if npc["iNPCType"].as_i64() != Some(i64::from(warp.npc_type))
                || npc["bHidden"].as_bool().unwrap_or(false)
                || !npc["bInteractable"].as_bool().unwrap_or(true) { continue; }
            let from = npc["iMapNum"].as_u64().unwrap_or(0) as u32;
            assert!(from == 0 || maps.contains(&from), "Spawn refers to undefined map {from}");
            links.entry(from).or_default().insert(warp.map_num);
        }
    }
    fn reachable(links: &HashMap<u32, HashSet<u32>>, start: u32) -> HashSet<u32> {
        let mut seen = HashSet::from([start]);
        let mut pending = vec![start];
        while let Some(current) = pending.pop() {
            for next in links.get(&current).into_iter().flatten() {
                if seen.insert(*next) { pending.push(*next); }
            }
        }
        seen
    }
    let from_world = reachable(&links, 0);
    for map in maps {
        assert!(from_world.contains(&map), "Map {map} has no placed production entrance");
        if [2, 4, 5, 8, 18, 130, 136, 142, 143, 144, 149, 151, 152].contains(&map) {
            assert!(reachable(&links, map).contains(&0), "Repaired map {map} has no placed return route");
        }
    }
}

#[test]
fn additional_instance_choices_preserve_original_access_and_consumption_rules() {
    let tables = tdata_init().unwrap();
    for (added, donor) in [(321, 5), (322, 291), (323, 319)] {
        let a = tables.get_warp_data(added).unwrap();
        let d = tables.get_warp_data(donor).unwrap();
        assert_ne!(a.map_num, d.map_num);
        assert_eq!(a.npc_type, d.npc_type);
        assert_eq!(a.pos, d.pos);
        assert_eq!(a.min_level, d.min_level);
        assert_eq!(a.req_task, d.req_task);
        assert_eq!(a.req_item, d.req_item);
        assert_eq!(a.req_item_count, d.req_item_count);
        assert_eq!(a.req_item_consumed, d.req_item_consumed);
        assert_eq!(a.req_completed_mission, d.req_completed_mission);
        assert_eq!(a.cost, d.cost);
        assert_eq!(a.is_group_warp, d.is_group_warp);
        assert_eq!(a.is_instance, d.is_instance);
    }
    let xdt = source("xdt.json");
    assert_eq!(tables.get_warp_data(317).unwrap().map_num, 149);
    for task in xdt["m_pMissionTable"]["m_pMissionData"].as_array().unwrap() {
        if [5236, 5237].contains(&task["m_iHTaskID"].as_i64().unwrap()) {
            assert_eq!(task["m_iRequireInstanceID"], 149,
                "Nano843 must require the same preserved map identity as its real portal");
        }
    }
}
