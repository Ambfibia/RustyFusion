//! Production 0104 weapon transactions against the accepted weapon table.
use super::combat::{pc_attack_npcs, pc_attack_pcs, pc_grenade_style_fire, pc_projectile_hit, pc_rocket_style_fire};
use crate::{chunk::TickMode, defines::*, entity::{Combatant, Entity, EntityID, Player, NPC}, enums::{ItemLocation, ItemType, WeaponTargetMode}, item::Item,
    net::{packet::{Packet, PacketBuilder, PacketID::*, *}, ClientMap, ClientMessage, ClientMetadata, ClientType, FFClient}, state::ShardServerState, tabledata::{tdata_get,tdata_init}, Position};
use std::{collections::HashMap, time::{Duration,SystemTime}};
type Rx=tokio::sync::mpsc::UnboundedReceiver<ClientMessage>;
#[repr(C)] #[derive(Debug,Copy,Clone)] struct Words<const N:usize>([i32;N]);
impl<const N:usize> FFPacket for Words<N> {}
fn packet<const N:usize>(id:PacketID,words:[i32;N])->Packet {Packet::new(id,&Words(words)).unwrap()}
fn fixture(weapon:Option<i16>,gm:bool)->(ShardServerState,HashMap<usize,FFClient>,Rx) {
    tdata_init().unwrap(); let(tx,rx)=tokio::sync::mpsc::unbounded_channel();
    let client=FFClient::new(tx,ClientMetadata::new("127.0.0.1:39021".parse().unwrap(),Some(ClientType::GameClient {account_id:1,serial_key:1,pc_id:Some(1)})));
    let mut player=Player::new(1,0); player.set_player_id(1); player.set_client(client.clone()); player.set_weapon_boosts(1000);
    if gm {player.perms=CN_ACCOUNT_LEVEL__GM as i16;}
    if let Some(id)=weapon {player.set_item(ItemLocation::Equip,EQUIP_SLOT_HAND as usize,Some(Item::new(ItemType::Hand,id))).unwrap();}
    let mut second=Player::new(2,0); second.set_player_id(2); second.set_position(player.get_position()); second.instance_id=player.instance_id;
    let mut state=ShardServerState::default(); state.entity_map=crate::chunk::EntityMap::default();
    for id in 10..16 {state.entity_map.track(Box::new(NPC::new(id,59,player.get_position(),0,player.instance_id).unwrap()),TickMode::Always);}
    state.entity_map.track(Box::new(second),TickMode::Always); state.entity_map.track(Box::new(player),TickMode::Always);
    (state,HashMap::from([(1,client)]),rx)
}
fn ids(mode:WeaponTargetMode)->Vec<i16>{(1..2000).filter(|id|tdata_get().get_item_stats(*id,ItemType::Hand).is_ok_and(|s|s.target_mode==Some(mode))).collect()}
fn hp(s:&ShardServerState,id:i32)->i32{s.get_npc(id).unwrap().get_hp()}
fn fire(s:&mut ShardServerState,c:&HashMap<usize,FFClient>,mode:WeaponTargetMode)->crate::error::FFResult<()> {
    let p=s.get_player(1).unwrap().get_position(); let clients=ClientMap::new(1,c);
    match mode {WeaponTargetMode::Rocket=>pc_rocket_style_fire(packet(P_CL2FE_REQ_PC_ROCKET_STYLE_FIRE,[0,p.x,p.y,p.z,p.x+100,p.y,p.z]),&clients,s),
        _=>pc_grenade_style_fire(packet(P_CL2FE_REQ_PC_GRENADE_STYLE_FIRE,[0,p.x+100,p.y,p.z]),&clients,s)}
}
fn hit(s:&ShardServerState,targets:&[(i32,i32)],slot:i32)->Packet {
    let p=s.get_player(1).unwrap().get_position();
    let mut b=PacketBuilder::new(P_CL2FE_REQ_PC_ROCKET_STYLE_HIT).with(&Words([slot,p.x,p.y,p.z,targets.len() as i32]));
    for &(id,high) in targets {b.push(&Words([id,high]));}b.build().unwrap()
}
fn packets(rx:&mut Rx)->Vec<Packet>{std::iter::from_fn(||rx.try_recv().ok()).filter_map(|m|match m{ClientMessage::SendPacket(p)=>Some(p),_=>None}).collect()}

#[test] fn accepted_weapon_table_covers_every_family_and_native_flight_units(){
    tdata_init().unwrap();let mut counts=Vec::new();
    for mode in [WeaponTargetMode::Melee,WeaponTargetMode::Pistol,WeaponTargetMode::Shotgun,WeaponTargetMode::Sniper,WeaponTargetMode::Rocket,WeaponTargetMode::Grenade] {
        let weapons=ids(mode);counts.push(weapons.len());
        for id in weapons {let s=tdata_get().get_item_stats(id,ItemType::Hand).unwrap();assert!(s.attack_range>0&&s.target_count>0&&s.attack_delay>Duration::ZERO,"{id}");
            if matches!(mode,WeaponTargetMode::Rocket|WeaponTargetMode::Grenade){let duration=s.projectile_time.unwrap();assert!(duration>=Duration::from_secs(1)&&duration<=Duration::from_millis(2500),"native duration is tenths of seconds, weapon {id}");if mode==WeaponTargetMode::Grenade{assert_eq!(duration,Duration::from_millis(2500));}assert!(s.effect_area>0);}}
    }assert_eq!(counts,[202,178,226,73,74,53]);
}
#[test] fn every_accepted_hitscan_weapon_uses_server_damage_cost_and_cooldown(){
    tdata_init().unwrap();
    let(mut s,c,mut rx)=fixture(None,false);
    for mode in [WeaponTargetMode::Melee,WeaponTargetMode::Pistol,WeaponTargetMode::Shotgun,WeaponTargetMode::Sniper] {for id in ids(mode){
        let player=s.get_player_mut(1).unwrap();player.set_item(ItemLocation::Equip,0,Some(Item::new(ItemType::Hand,id))).unwrap();player.last_weapon_attack=None;player.set_weapon_boosts(1000);s.get_npc_mut(10).unwrap().heal(350);
        let request=packet(P_CL2FE_REQ_PC_ATTACK_NPCs,[1,10]);
        pc_attack_npcs(request.clone(),&ClientMap::new(1,&c),&mut s).unwrap();assert!(hp(&s,10)<350,"{id}");
        let battery=s.get_player(1).unwrap().get_weapon_boosts();assert_eq!(battery,1000-6-s.get_npc(10).unwrap().get_level() as u32);
        assert!(pc_attack_npcs(request,&ClientMap::new(1,&c),&mut s).is_err(),"{id}");assert_eq!(s.get_player(1).unwrap().get_weapon_boosts(),battery);
        assert!(packets(&mut rx).iter().any(|p|p.id()==P_FE2CL_PC_ATTACK_NPCs_SUCC));
    }}
}
#[test] fn every_accepted_projectile_weapon_fires_hits_once_and_preserves_cost(){
    let(mut s,c,mut rx)=fixture(None,false);for mode in [WeaponTargetMode::Rocket,WeaponTargetMode::Grenade]{for id in ids(mode){
        let player=s.get_player_mut(1).unwrap();player.set_item(ItemLocation::Equip,0,Some(Item::new(ItemType::Hand,id))).unwrap();player.last_weapon_attack=None;player.set_weapon_boosts(1000);s.get_npc_mut(10).unwrap().heal(350);
        fire(&mut s,&c,mode).unwrap();assert_eq!(s.get_player(1).unwrap().projectile_count(),1);
        let battery=1000-6-tdata_get().get_item_stats(id,ItemType::Hand).unwrap().required_level as u32;assert_eq!(s.get_player(1).unwrap().get_weapon_boosts(),battery);
        let slot=(0..3).find(|slot|s.get_player(1).unwrap().get_projectile(*slot).is_some()).unwrap();
        let request=hit(&s,&[(10,0)],i32::from(slot));pc_projectile_hit(request.clone(),&ClientMap::new(1,&c),&mut s).unwrap();let after=hp(&s,10);assert!(after<350,"{id}");
        assert!(pc_projectile_hit(request,&ClientMap::new(1,&c),&mut s).is_err());assert_eq!(hp(&s,10),after);assert_eq!(s.get_player(1).unwrap().get_weapon_boosts(),battery);
        let expected=if mode==WeaponTargetMode::Rocket{P_FE2CL_REP_PC_ROCKET_STYLE_FIRE_SUCC}else{P_FE2CL_REP_PC_GRENADE_STYLE_FIRE_SUCC};assert!(packets(&mut rx).iter().any(|p|p.id()==expected));
    }}
}
#[test] fn ordinary_accounts_cannot_attack_players_and_gm_uses_eight_byte_namespaces(){
    for gm in [false,true] {let(mut s,c,mut rx)=fixture(None,gm);let before=s.get_player(2).unwrap().get_hp();
        let result=pc_attack_pcs(packet(P_CL2FE_REQ_PC_ATTACK_CHARs,[1,2,1]),&ClientMap::new(1,&c),&mut s);
        assert_eq!(result.is_ok(),gm);assert_eq!(s.get_player(2).unwrap().get_hp()<before,gm);
        assert_eq!(packets(&mut rx).iter().any(|p|p.id()==P_FE2CL_PC_ATTACK_CHARs_SUCC),gm);
    }
    for words in [[1,2,4],[1,2,99],[1,1,1]] {let(mut s,c,_)=fixture(None,true);assert!(pc_attack_pcs(packet(P_CL2FE_REQ_PC_ATTACK_CHARs,words),&ClientMap::new(1,&c),&mut s).is_err());assert_eq!(s.get_player(1).unwrap().get_weapon_boosts(),1000);}
    let(mut s,c,_)=fixture(None,true);assert!(pc_attack_pcs(packet(P_CL2FE_REQ_PC_ATTACK_CHARs,[1,2]),&ClientMap::new(1,&c),&mut s).is_err());
}
#[test] fn invalid_targets_do_not_spend_battery_or_hold_cooldown(){
    for case in ["far","instance","friendly","dead","duplicate","trailing"] {let(mut s,c,_)=fixture(None,false);let mut request=packet(P_CL2FE_REQ_PC_ATTACK_NPCs,[1,10]);
        match case {"far"=>s.get_npc_mut(10).unwrap().set_position(Position::new(100000,100000,0)),"instance"=>s.get_npc_mut(10).unwrap().instance_id.channel_num=2,
            "friendly"=>s.get_npc_mut(10).unwrap().ty=637,"dead"=>{s.get_combatant_mut(EntityID::NPC(10)).unwrap().take_damage(350,None);},
            "duplicate"=>request=packet(P_CL2FE_REQ_PC_ATTACK_NPCs,[2,10,10]),_=>request=packet(P_CL2FE_REQ_PC_ATTACK_NPCs,[1,10,11])}
        assert!(pc_attack_npcs(request,&ClientMap::new(1,&c),&mut s).is_err(),"{case}");let p=s.get_player(1).unwrap();assert_eq!(p.get_weapon_boosts(),1000);assert!(p.last_weapon_attack.is_none());
    }
}
#[test] fn projectile_bad_hits_keep_ownership_until_valid_commit_and_expiry_reclaims_slots(){
    tdata_init().unwrap();for case in ["duplicate","highword","instance","farblast","path","count","expired"]{
        let(mut s,c,_)=fixture(Some(ids(WeaponTargetMode::Rocket)[0]),false);fire(&mut s,&c,WeaponTargetMode::Rocket).unwrap();let mut request=hit(&s,&[(10,0)],0);
        match case {"duplicate"=>request=hit(&s,&[(10,0),(10,0)],0),"highword"=>request=hit(&s,&[(10,1)],0),
            "instance"=>s.get_npc_mut(10).unwrap().instance_id.channel_num=2,"farblast"=>s.get_npc_mut(10).unwrap().set_position(Position::new(9999,9999,0)),
            "path"=>request=packet(P_CL2FE_REQ_PC_ROCKET_STYLE_HIT,[0,0,1000,0,1,10,0]),"count"=>request=hit(&s,&[(10,0),(11,0),(12,0),(13,0),(14,0),(15,0)],0),
            _=>{let mut p=s.get_player(1).unwrap().get_projectile(0).unwrap();p.expires=SystemTime::UNIX_EPOCH;s.get_player_mut(1).unwrap().remove_projectile(0);s.get_player_mut(1).unwrap().add_projectile(p);}}
        assert!(pc_projectile_hit(request,&ClientMap::new(1,&c),&mut s).is_err(),"{case}");assert_eq!(hp(&s,10),350);assert_eq!(s.get_player(1).unwrap().projectile_count(),if case=="expired"{0}else{1});
    }
}
#[test] fn projectile_launch_validates_mode_origin_skill_and_capacity_before_charge(){
    tdata_init().unwrap();let id=ids(WeaponTargetMode::Rocket)[0];
    for words in [[1,0,0,0,0,0,0],[0,5000,0,0,0,0,0],[0,0,0,0,99999,0,0]] {let(mut s,c,_)=fixture(Some(id),false);assert!(pc_rocket_style_fire(packet(P_CL2FE_REQ_PC_ROCKET_STYLE_FIRE,words),&ClientMap::new(1,&c),&mut s).is_err());assert_eq!(s.get_player(1).unwrap().get_weapon_boosts(),1000);}
    let(mut s,c,_)=fixture(Some(id),false);assert!(fire(&mut s,&c,WeaponTargetMode::Grenade).is_err());
    for _ in 0..3 {s.get_player_mut(1).unwrap().last_weapon_attack=None;fire(&mut s,&c,WeaponTargetMode::Rocket).unwrap();}
    s.get_player_mut(1).unwrap().last_weapon_attack=None;let before=s.get_player(1).unwrap().get_weapon_boosts();assert!(fire(&mut s,&c,WeaponTargetMode::Rocket).is_err());assert_eq!(s.get_player(1).unwrap().get_weapon_boosts(),before);
    let p=s.get_player_mut(1).unwrap();let mut instance=p.instance_id;instance.channel_num+=1;p.set_instance_id(instance);assert_eq!(p.projectile_count(),0);
}
