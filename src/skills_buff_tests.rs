//! Regression coverage for effective-stack lifetime, aura ownership, reward
//! rates, Phoenix authorization and the accepted 36-byte Leech result.
use super::*;
use crate::{chunk::TickMode, entity::{Group, Player, NPC}, enums::{RewardCategory, RewardType}, tabledata::{tdata_get, tdata_init}};

fn buff(value: i32) -> BuffInstance { BuffInstance::new(BuffType::Nano, value, None, None, None) }
fn player(id: i32) -> Player { let mut p = Player::new(id as i64, 0); p.set_player_id(id); p.set_level(36).unwrap(); p.heal(p.get_max_hp()); p }
#[test]
fn expired_strong_stack_never_procs_and_reports_surviving_value() {
    let mut container = BuffContainer::default();
    container.add_buff(BuffID::BoundingBall, buff(10), None);
    let mut expired = buff(100); expired.expires = Some(Instant::now() - Duration::from_secs(1));
    container.add_buff(BuffID::BoundingBall, expired, None);
    assert_eq!(container.get_buff_value(BuffID::BoundingBall), Some(10));
    let mut effects = Vec::new(); let updates = container.tick(EntityID::Player(1), &mut effects);
    assert_eq!(effects.len(), 1);
    assert!(matches!(updates[0], BuffUpdate::Added(BuffID::BoundingBall, _, value) if value.iValue == 10));
    container.buff_stacks.get_mut(&BuffID::BoundingBall).unwrap().buffs[0].expires = Some(Instant::now() - Duration::from_secs(1));
    assert!(!container.has_buff(BuffID::BoundingBall, None)); assert_eq!(container.get_bit_flags(), 0);
    effects.clear(); let updates = container.tick(EntityID::Player(1), &mut effects);
    assert!(effects.is_empty()); assert!(matches!(updates[0], BuffUpdate::Removed(BuffID::BoundingBall)));
}
#[test]
fn expired_strong_stack_emits_change_to_weaker_still_live_stack() {
    let mut container = BuffContainer::default(); container.add_buff(BuffID::UpMoveSpeed, buff(100), None);
    container.add_buff(BuffID::UpMoveSpeed, buff(200), None);
    container.tick(EntityID::Player(1), &mut Vec::new());
    container.buff_stacks.get_mut(&BuffID::UpMoveSpeed).unwrap().buffs[1].expires = Some(Instant::now() - Duration::from_secs(1));
    let updates = container.tick(EntityID::Player(1), &mut Vec::new());
    assert!(matches!(updates[0], BuffUpdate::Changed(BuffID::UpMoveSpeed, _, value) if value.iValue == 100));
}
#[test]
fn group_run_aura_is_source_owned_and_removes_on_map_leave_without_removing_active_cast() {
    tdata_init().unwrap(); let mut state = ShardServerState::default();
    let gid = uuid::Uuid::new_v4(); let mut a = player(1); let mut b = player(2);
    a.group_id = Some(gid); b.group_id = Some(gid);
    a.unlock_nano(3).unwrap().selected_skill = Some(8); a.change_nano(0, Some(3)).unwrap(); a.activate_nano(0).unwrap();
    b.apply_buff(BuffID::UpMoveSpeed, BuffInstance::new(BuffType::GroupNano, 50, None, None, None), Some(EntityID::Player(90)));
    let mut group = Group::new(EntityID::Player(1)); group.add_member(EntityID::Player(2)).unwrap(); state.groups.insert(gid, group);
    state.entity_map.track(Box::new(a), TickMode::Always); state.entity_map.track(Box::new(b), TickMode::Always);
    refresh_group_auras(&mut state);
    let speed = tdata_get().get_skill(8).unwrap().values_a[0];
    assert_eq!(state.get_player(2).unwrap().get_speed(true), PLAYER_RUN_SPEED + speed);
    state.get_player_mut(1).unwrap().set_instance_id(InstanceID { map_num: 2, ..InstanceID::default() });
    refresh_group_auras(&mut state);
    assert_eq!(state.get_player(2).unwrap().get_speed(true), PLAYER_RUN_SPEED + 50);
}
#[test]
fn reward_nano_bonus_multiplies_existing_rates_without_boosting_eggs() {
    tdata_init().unwrap(); let mut p = player(1);
    p.reward_data.set_reward_rate(RewardType::Taros, RewardCategory::Combat, 150.0);
    p.apply_buff(BuffID::RewardCash, buff(200), None);
    assert!((p.effective_reward_data().get_reward_rate(RewardType::Taros, 1).unwrap() - 1.8).abs() < 0.0001);
    assert_eq!(p.effective_reward_data().get_reward_rate(RewardType::Taros, 3).unwrap(), 1.0);
    assert_eq!(p.reward_data.get_reward_rate(RewardType::Taros, 1).unwrap(), 1.5);
}

#[test]
fn tired_nano_switch_preserves_the_current_passive_until_the_new_nano_can_summon() {
    tdata_init().unwrap();
    let mut rider = player(1);
    rider.unlock_nano(8).unwrap().selected_skill = Some(22);
    rider.unlock_nano(3).unwrap().selected_skill = Some(8);
    rider.change_nano(0, Some(8)).unwrap();
    rider.change_nano(1, Some(3)).unwrap();
    rider.activate_nano(0).unwrap();

    let threshold = NANO_STAMINA_MAX / 5;
    rider.get_nano_mut(3).unwrap().set_stamina(threshold);
    assert!(rider.activate_nano(1).is_err());
    assert_eq!(rider.get_active_nano_slot(), Some(0));
    assert!(rider.has_buff(BuffID::Phoenix, Some(BuffType::Nano)));

    rider.get_nano_mut(3).unwrap().set_stamina(threshold + 1);
    rider.activate_nano(1).unwrap();
    assert_eq!(rider.get_active_nano_slot(), Some(1));
    assert!(!rider.has_buff(BuffID::Phoenix, Some(BuffType::Nano)));
}
#[test]
fn phoenix_requires_real_buff_dead_target_and_consumes_only_active_nano() {
    tdata_init().unwrap(); let mut p = player(1);
    p.unlock_nano(8).unwrap().selected_skill = Some(22); p.unlock_nano(3).unwrap();
    p.change_nano(0, Some(8)).unwrap(); p.change_nano(1, Some(3)).unwrap(); p.activate_nano(0).unwrap();
    assert!(p.do_phoenix_revive(false).is_err());
    p.take_damage(p.get_max_hp(), None); p.do_phoenix_revive(false).unwrap();
    assert_eq!(p.get_hp(), p.get_max_hp() / 2); assert_eq!(p.get_nano(8).unwrap().get_stamina(), 0);
    assert_eq!(p.get_nano(3).unwrap().get_stamina(), NANO_STAMINA_MAX);
    p.take_damage(p.get_max_hp(), None); assert!(p.do_phoenix_revive(false).is_err()); assert!(p.do_phoenix_revive(true).is_err());
}
#[test]
fn group_phoenix_cast_authorizes_one_revive_then_consumes_authorization() {
    tdata_init().unwrap(); let mut state = ShardServerState::default(); let a = player(1); let mut b = player(2);
    b.take_damage(b.get_max_hp(), None); state.entity_map.track(Box::new(a), TickMode::Always); state.entity_map.track(Box::new(b), TickMode::Always);
    assert!(state.get_player_mut(2).unwrap().do_phoenix_revive(true).is_err());
    let results = do_skill(EntityID::Player(1), &[EntityID::Player(2)], tdata_get().get_skill(20).unwrap(), 0, &mut state).unwrap();
    assert!(matches!(results[0], SkillResult::Resurrect(r) if r.iRegenHP > 0));
    let p = state.get_player_mut(2).unwrap(); p.do_phoenix_revive(true).unwrap(); p.take_damage(p.get_max_hp(), None);
    assert!(p.do_phoenix_revive(true).is_err());
}
#[test]
fn leech_serializes_capped_caster_heal_and_target_damage_in_exact_native_order() {
    tdata_init().unwrap(); let mut state = ShardServerState::default(); let mut a = player(1); let pos = a.get_position(); a.take_damage(1, None);
    state.entity_map.track(Box::new(a), TickMode::Always);
    state.entity_map.track(Box::new(NPC::new(2000000194, 59, pos, 0, InstanceID::default()).unwrap()), TickMode::Always);
    let results = do_skill(EntityID::Player(1), &[EntityID::NPC(2000000194)], tdata_get().get_skill(24).unwrap(), 0, &mut state).unwrap();
    let SkillResult::Leech(leech) = &results[0] else { panic!("Wrong Leech ABI"); };
    assert_eq!(size_of::<SkillResultLeech>(), 36); let heal = leech.heal; let damage = leech.damage;
    assert_eq!(({heal.eCT}, {heal.iID}, {heal.iHealHP}), (1, 1, 1)); assert_eq!({damage.iID}, 2000000194); assert!({damage.iDamage} > 0);
}
