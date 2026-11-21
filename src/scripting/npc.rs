use mlua::prelude::*;
use rand::thread_rng;

use crate::{
    corruption,
    entity::{Combatant, Entity, EntityID, NPC},
    error::log_if_failed,
    helpers, skills,
    path::Path,
    tabledata::tdata_get,
    Position,
};

use super::{with_active_shard, EntityScriptContext};

/// NPC context used in Lua script. Provides NPC bindings to Lua.
#[derive(Debug, Clone, Copy)]
pub(super) struct NpcScriptContext {
    npc_id: i32,
}
impl NpcScriptContext {
    pub(super) fn new(npc_id: i32) -> Self {
        Self { npc_id }
    }

    fn with_npc<T>(&self, lua: &Lua, f: impl FnOnce(&mut NPC) -> LuaResult<T>) -> LuaResult<T> {
        with_active_shard(lua, |state| {
            let npc = state.get_npc_mut(self.npc_id)?;
            f(npc)
        })
    }
}

impl FromLua for NpcScriptContext {
    fn from_lua(value: LuaValue, _: &Lua) -> LuaResult<Self> {
        match value {
            LuaValue::UserData(ud) => {
                let npc = ud.borrow::<Self>()?;
                Ok(*npc)
            }
            _ => Err(LuaError::runtime("expected Npc")),
        }
    }
}
impl LuaUserData for NpcScriptContext {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        luau_class!("Npc" extends "Entity", {
            // Properties

            luau_method!(methods, "id" -> "number", |_, this, ()| Ok(this.npc_id));

            luau_method!(methods, "ty" -> "number", |lua, this, ()| this.with_npc(lua, |npc| {
                Ok(npc.ty)
            }));

            luau_method!(methods, "spawn_position" -> "Position", |lua, this, ()| this.with_npc(lua, |npc| {
                Ok(npc.spawn_position)
            }));

            luau_method!(methods, "retreating" -> "boolean", |lua, this, ()| this.with_npc(lua, |npc| {
                Ok(npc.retreating)
            }));

            luau_method!(methods, "is_moving" -> "boolean", |lua, this, ()| this.with_npc(lua, |npc| {
                Ok(npc.path.is_some())
            }));
            luau_method!(methods, "has_authored_path" -> "boolean", |lua, this, ()| this.with_npc(lua, |npc| {
                Ok(npc.authored_path.is_some())
            }));

            // Stats

            luau_method!(methods, "walk_speed" -> "number", |lua, this, ()| this.with_npc(lua, |npc| {
                let stats = tdata_get()
                    .get_npc_stats(npc.ty)
                    .unwrap();

                Ok(stats.walk_speed)
            }));

            luau_method!(methods, "run_speed" -> "number", |lua, this, ()| this.with_npc(lua, |npc| {
                let stats = tdata_get()
                    .get_npc_stats(npc.ty)
                    .unwrap();

                Ok(stats.run_speed)
            }));

            luau_method!(methods, "sight_range" -> "number", |lua, this, ()| this.with_npc(lua, |npc| {
                let stats = tdata_get()
                    .get_npc_stats(npc.ty)
                    .unwrap();

                Ok(stats.sight_range)
            }));

            luau_method!(methods, "idle_range" -> "number", |lua, this, ()| this.with_npc(lua, |npc| {
                let stats = tdata_get()
                    .get_npc_stats(npc.ty)
                    .unwrap();

                Ok(stats.idle_range)
            }));

            luau_method!(methods, "combat_range" -> "number", |lua, this, ()| this.with_npc(lua, |npc| {
                let stats = tdata_get()
                    .get_npc_stats(npc.ty)
                    .unwrap();

                Ok(stats.combat_range)
            }));

            luau_method!(methods, "attack_range" -> "number", |lua, this, ()| this.with_npc(lua, |npc| {
                let stats = tdata_get()
                    .get_npc_stats(npc.ty)
                    .map_err(|e| LuaError::runtime(e.to_string()))?;

                Ok(stats.attack_range)
            }));

            luau_method!(methods, "regen_time" -> "number", |lua, this, ()| this.with_npc(lua, |npc| {
                let stats = tdata_get()
                    .get_npc_stats(npc.ty)
                    .map_err(|e| LuaError::runtime(e.to_string()))?;

                // Convert from 100ms units to seconds
                Ok(stats.regen_time as f64 / 10.0)
            }));

            luau_method!(methods, "delay_time" -> "number", |lua, this, ()| this.with_npc(lua, |npc| {
                let stats = tdata_get()
                    .get_npc_stats(npc.ty)
                    .map_err(|e| LuaError::runtime(e.to_string()))?;

                // Convert from 100ms units to seconds
                Ok(stats.delay_time as f64 / 10.0)
            }));

            // Actions

            luau_method!(methods, "clear_target" -> "()", |lua, this, ()| this.with_npc(lua, |npc| {
                npc.target_id = None;
                Ok(())
            }));

            luau_method!(methods, "set_target" -> "()", |lua, this, target: EntityScriptContext| this.with_npc(lua, |npc| {
                npc.acquire_target(target.id());
                Ok(())
            }));

            luau_method!(methods, "combat_origin" -> "Position", |lua, this, ()| this.with_npc(lua, |npc| {
                Ok(npc.combat_origin.unwrap_or(npc.spawn_position))
            }));

            luau_method!(methods, "attack" -> "()", |lua, this, ()| with_active_shard(lua, |state| {
                let npc = state.get_npc(this.npc_id)?;
                let target_id = match npc.target_id {
                    Some(id) => id,
                    None => return Err(LuaError::runtime("No target to attack")),
                };

                skills::do_basic_attack(
                    EntityID::NPC(this.npc_id),
                    &[target_id],
                    false,
                    skills::AttackContext::default(),
                    state,
                )?;
                Ok(())
            }));

            luau_method!(methods, "try_begin_corruption" -> "boolean", |lua, this, ()| with_active_shard(lua, |state| {
                Ok(corruption::try_begin(this.npc_id, state)?)
            }));

            luau_method!(methods, "corruption_interrupted" -> "boolean", |lua, this, ()| with_active_shard(lua, |state| {
                Ok(corruption::is_interrupted(this.npc_id, state))
            }));

            luau_method!(methods, "finish_corruption" -> "boolean", |lua, this, ()| with_active_shard(lua, |state| {
                Ok(corruption::finish(this.npc_id, state)?)
            }));
            luau_method!(methods, "try_active_skill" -> "boolean", |lua, this, ()| with_active_shard(lua, |state| {
                Ok(crate::ai::try_active_skill(this.npc_id, state)?)
            }));
            luau_method!(methods, "try_begin_eruption" -> "boolean", |lua, this, ()| with_active_shard(lua, |state| {
                Ok(crate::eruption::try_begin(this.npc_id, state)?)
            }));
            luau_method!(methods, "eruption_interrupted" -> "boolean", |lua, this, ()| with_active_shard(lua, |state| {
                Ok(crate::eruption::is_interrupted(this.npc_id, state))
            }));
            luau_method!(methods, "finish_eruption" -> "boolean", |lua, this, ()| with_active_shard(lua, |state| {
                Ok(crate::eruption::finish(this.npc_id, state)?)
            }));

            luau_method!(methods, "move_to" -> "()",
                |lua, this, (x, y, z, speed): (i32, i32, i32, Option<i32>)| with_active_shard(lua, |state| {
                    let target_pos = Position { x, y, z };
                    let npc = state.get_npc_mut(this.npc_id)?;
                    npc.move_towards(target_pos, speed);
                    Ok(())
                })
            );

            luau_method!(methods, "move_toward_entity" -> "()", |lua, this, (target, speed): (EntityScriptContext, Option<i32>)| with_active_shard(lua, |state| {
                let target_pos = match state.entity_map.get_entity_raw(target.id()) {
                    Some(entity) => entity.get_position(),
                    None => return Err(LuaError::runtime("Entity not found")),
                };

                let npc = state.get_npc_mut(this.npc_id)?;
                npc.move_towards(target_pos, speed);
                Ok(())
            }));

            // Formation offsets already account for spacing. Applying the
            // combat collision radius here leaves followers behind their slot.
            luau_method!(methods, "move_to_exact" -> "()",
                |lua, this, (x, y, z, speed): (i32, i32, i32, i32)| this.with_npc(lua, |npc| {
                    let mut path = Path::new_single(Position { x, y, z }, speed);
                    path.start();
                    npc.set_path(path);
                    Ok(())
                })
            );

            luau_method!(methods, "stop" -> "()", |lua, this, ()| this.with_npc(lua, |npc| {
                npc.path = None;
                Ok(())
            }));

            luau_method!(methods, "set_retreating" -> "()", |lua, this, retreating: bool| this.with_npc(lua, |npc| {
                npc.retreating = retreating;
                Ok(())
            }));

            luau_method!(methods, "return_home" -> "()", |lua, this, ()| with_active_shard(lua, |state| {
                Ok(crate::ai::return_home(this.npc_id, state)?)
            }));

            luau_method!(methods, "begin_death" -> "()", |lua, this, ()| with_active_shard(lua, |state| {
                // a corpse keeps no conditions or windup (OpenFusion MobAI::onDeath)
                state.get_npc_mut(this.npc_id)?.clear_buffs();
                corruption::cancel(this.npc_id, state)?;
                crate::eruption::cancel(this.npc_id, state)?;
                let last_attacked_by = state.get_npc(this.npc_id)?.last_attacked_by;
                if let Some(defeater_id) = last_attacked_by {
                    let mut rng = thread_rng();
                    log_if_failed(helpers::on_mob_defeated(
                        this.npc_id,
                        defeater_id,
                        state,
                        &mut rng,
                    ));
                }
                Ok(())
            }));

            luau_method!(methods, "despawn" -> "()", |lua, this, ()| with_active_shard(lua, |state| {
                let entity_id = EntityID::NPC(this.npc_id);
                let summoned = state.get_npc(this.npc_id)?.summoned;
                state.entity_map.update(entity_id, None, true);
                if summoned {
                    state.entity_map.mark_for_cleanup(entity_id);
                }
                Ok(())
            }));

            luau_method!(methods, "respawn" -> "()", |lua, this, ()| with_active_shard(lua, |state| {
                let chunk_pos = {
                    let npc = state.get_npc_mut(this.npc_id)?;
                    npc.reset();
                    npc.set_position(npc.spawn_position);
                    npc.get_chunk_coords()
                };
                state.entity_map.update(EntityID::NPC(this.npc_id), Some(chunk_pos), true);
                Ok(())
            }));

            luau_method!(methods, "summon_npc" -> "number",
                |lua, this, (ty, offset_x, offset_y, offset_z): (i32, i32, i32, i32)| with_active_shard(lua, |state| {
                    tdata_get()
                        .get_npc_stats(ty)
                        .map_err(|e| LuaError::runtime(e.to_string()))?;

                    let (pos, angle, instance_id) = {
                        let summoner = state.get_npc(this.npc_id)?;
                        (
                            summoner.get_position(),
                            summoner.get_rotation(),
                            summoner.instance_id,
                        )
                    };
                    let spawn_pos = Position {
                        x: pos.x + offset_x,
                        y: pos.y + offset_y,
                        z: pos.z + offset_z,
                    };

                    let entity_map = &mut state.entity_map;
                    let npc_id = entity_map.gen_next_npc_id();
                    let npc = NPC::new(npc_id, ty, spawn_pos, angle, instance_id)
                        .map_err(|e| LuaError::runtime(e.to_string()))?;
                    helpers::spawn_temp_npc(entity_map, npc);
                    Ok(npc_id)
                })
            );

            luau_method!(methods, "set_spawn_position" -> "()",
                |lua, this, (x, y, z): (i32, i32, i32)| this.with_npc(lua, |npc| {
                    npc.spawn_position = Position { x, y, z };
                    Ok(())
                })
            );

            // Helpers

            luau_method!(methods, "in_attack_range" -> "boolean", |lua, this, ()| with_active_shard(lua, |state| {
                let npc = state.get_npc(this.npc_id)?;
                let target_id = match npc.target_id {
                    Some(id) => id,
                    None => return Ok(false),
                };
                let stats = tdata_get()
                    .get_npc_stats(npc.ty)
                    .map_err(|e| LuaError::runtime(e.to_string()))?;
                let attack_range = stats.attack_range + stats.radius;
                let npc_pos = npc.get_position();
                let target = match state.entity_map.get_entity_raw(target_id) {
                    Some(entity) => entity,
                    None => return Ok(false),
                };
                Ok(npc_pos.distance_to(&target.get_position()) <= attack_range)
            }));

            luau_method!(methods, "find_nearest_enemy" -> "Entity?", |lua, this, range: u32| with_active_shard(lua, |state| {
                let (npc_team, npc_pos) = {
                    let npc = state.get_npc(this.npc_id)?;
                    (npc.get_team(), npc.get_position())
                };

                let mut nearest_id: Option<EntityID> = None;
                let mut nearest_dist = u32::MAX;

                for eid in state.entity_map.get_around_entity(EntityID::NPC(this.npc_id)) {
                    let entity = match state.entity_map.get_entity_raw(eid) {
                        Some(e) => e,
                        None => continue,
                    };
                    let cb = match entity.as_combatant() {
                        Some(cb) => cb,
                        None => continue,
                    };
                    if cb.is_dead() || cb.get_team() == npc_team || cb.get_aggro_factor() <= 0.0 {
                        continue;
                    }
                    let dist = npc_pos.distance_to(&cb.get_position());
                    if dist <= range && dist < nearest_dist {
                        nearest_dist = dist;
                        nearest_id = Some(eid);
                    }
                }

                match nearest_id {
                    Some(eid) => Ok(Some(EntityScriptContext::new(eid))),
                    None => Ok(None),
                }
            }));

            luau_method!(methods, "get_follow_target" -> "Entity?", |lua, this, ()| with_active_shard(lua, |state| {
                let loose_follow = state.get_npc(this.npc_id)?.loose_follow;
                match loose_follow {
                    Some(eid) => Ok(Some(EntityScriptContext::new(eid))),
                    None => Ok(None),
                }
            }));

            luau_method!(methods, "get_pack_leader" -> "Npc?", |lua, this, ()| with_active_shard(lua, |state| {
                let tight_follow = state.get_npc(this.npc_id)?.tight_follow;
                let leader_id = match tight_follow {
                    Some((leader_id, _)) => leader_id,
                    None => return Ok(None),
                };

                let leader_npc_id = match leader_id {
                    EntityID::NPC(npc_id) => npc_id,
                    _ => return Ok(None),
                };

                Ok(Some(NpcScriptContext::new(leader_npc_id)))
            }));

            luau_method!(methods, "get_pack_offset" -> "Position", |lua, this, ()| this.with_npc(lua, |npc| {
                let offset = npc.tight_follow
                    .map(|(_, off)| off)
                    .unwrap_or_default();

                let table = lua.create_table()?;
                table.set("x", offset.x)?;
                table.set("y", offset.y)?;
                table.set("z", offset.z)?;
                Ok(table)
            }));

            luau_method!(methods, "find_enemies_in_range" -> "{Entity}", |lua, this, range: u32| with_active_shard(lua, |state| {
                let (npc_team, npc_pos) = {
                    let npc = state.get_npc(this.npc_id)?;
                    (npc.get_team(), npc.get_position())
                };

                let result = lua.create_table()?;
                let mut idx = 1;

                let nearby = state.entity_map.get_around_entity(EntityID::NPC(this.npc_id));
                for eid in nearby {
                    let in_range = {
                        let entity = match state.entity_map.get_entity_raw(eid) {
                            Some(e) => e,
                            None => continue,
                        };
                        let cb = match entity.as_combatant() {
                            Some(cb) => cb,
                            None => continue,
                        };
                        if cb.is_dead() || cb.get_team() == npc_team || cb.get_aggro_factor() <= 0.0 {
                            continue;
                        }
                        npc_pos.distance_to(&cb.get_position()) <= range
                    };
                    if in_range {
                        result.set(idx, EntityScriptContext::new(eid))?;
                        idx += 1;
                    }
                }

                Ok(result)
            }));

            methods.add_meta_method(LuaMetaMethod::Eq, |_, this, other: NpcScriptContext| {
                Ok(this.npc_id == other.npc_id)
            });

            methods.add_meta_method(LuaMetaMethod::ToString, |lua, this, ()| this.with_npc(lua, |npc| {
                Ok(format!("{}", npc))
            }));

            methods.add_meta_method(LuaMetaMethod::Index, |lua, _, key: String| {
                let wrapper = lua.create_function(move |lua, args: LuaMultiValue| {
                    let mut args_vec: Vec<_> = args.into_iter().collect();
                    let this_val = args_vec
                        .first()
                        .ok_or_else(|| LuaError::runtime("missing self"))?
                        .clone();

                    let npc = NpcScriptContext::from_lua(this_val, lua)?;
                    let entity = EntityScriptContext::new(EntityID::NPC(npc.npc_id));
                    let entity_ud = lua.create_userdata(entity)?;
                    let method: LuaFunction = entity_ud.get(key.as_str())?;
                    args_vec[0] = LuaValue::UserData(entity_ud);
                    method.call::<LuaMultiValue>(LuaMultiValue::from_vec(args_vec))
                })?;
                Ok(LuaValue::Function(wrapper))
            });
        }); // luau_class
    }
}
