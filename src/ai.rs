use crate::{
    chunk::TickMode,
    entity::{Entity, NPC},
    enums::CombatantTeam,
    tabledata::tdata_get,
};

/// Lord Fuse fights in three bodies. The first two summon the next stage (and
/// one of his arms) when they go down; the last one is just a mob.
pub const NPC_TYPE_FUSE_STAGE_1: i32 = 2466;
pub const NPC_TYPE_FUSE_STAGE_2: i32 = 2467;

fn get_script_name(npc: &NPC) -> Option<&'static str> {
    let is_combatant = npc.as_combatant().is_some();
    if !is_combatant {
        return None;
    }

    if matches!(npc.ty, NPC_TYPE_FUSE_STAGE_1 | NPC_TYPE_FUSE_STAGE_2) {
        return Some("lord_fuse");
    }

    let stats = tdata_get().get_npc_stats(npc.ty).unwrap();
    match stats.team {
        CombatantTeam::Friendly => Some("friendly_combatant"),
        CombatantTeam::Mob => {
            if npc.tight_follow.is_some() {
                Some("mob_pack_member")
            } else {
                Some("mob")
            }
        }
        _ => None,
    }
}

pub fn make_for_npc(npc: &NPC, force: bool) -> (Option<String>, TickMode) {
    let stats = tdata_get().get_npc_stats(npc.ty).unwrap();
    if !force && npc.path.is_none() && stats.ai_type == 0 {
        return (None, TickMode::Never);
    }

    let behavior_name = get_script_name(npc);
    let tick_mode = if npc.path.is_some() {
        TickMode::Always
    } else {
        TickMode::WhenLoaded
    };

    (behavior_name.map(|name| name.to_string()), tick_mode)
}
