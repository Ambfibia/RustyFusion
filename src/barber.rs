//! Server-authoritative, quoted appearance transactions for the 0104 barber.
use crate::{
    config::config_get,
    defines::{RANGE_INTERACT, SIZEOF_INVEN_SLOT},
    entity::{Combatant, EntityID, PlayerStyle},
    enums::ItemLocation,
    error::*,
    net::{
        packet::{PacketID::*, *},
        ClientMap, FFClient,
    },
    state::ShardServerState,
    tabledata::tdata_get,
};

#[derive(Debug, Clone, Copy)]
pub struct BarberSession {
    pub npc_id: i32,
    pub prices: BarberPrices,
}

fn fail(reason: &str) -> FFError {
    FFError::build(Severity::Warning, reason.to_owned())
}
fn validate(state: &ShardServerState, pc_id: i32, npc_id: i32) -> FFResult<()> {
    let npc = state.get_npc(npc_id)?;
    let player = state.get_player(pc_id)?;
    if npc.instance_id != player.instance_id
        || npc.get_hp() <= 0
        || player.get_hp() <= 0
        || player.trade_id.is_some()
        || tdata_get().get_npc_stats(npc.ty)?.service_category != 28
    {
        return Err(fail("Invalid barber owner or player state"));
    }
    state.entity_map.validate_proximity(
        &[EntityID::Player(pc_id), EntityID::NPC(npc_id)],
        RANGE_INTERACT,
    )
}
fn prices() -> FFResult<BarberPrices> {
    let c = &config_get().shard;
    let price = |n: u32| i32::try_from(n).map_err(|_| fail("Barber price exceeds wire range"));
    Ok(BarberPrices {
        change_gender: i32::from(c.barber_allow_gender.get()),
        gender_cost: price(c.barber_gender_cost.get())?,
        body_cost: price(c.barber_body_cost.get())?,
        skin_color_cost: price(c.barber_skin_cost.get())?,
        hair_style_cost: price(c.barber_hair_cost.get())?,
        hair_color_cost: price(c.barber_hair_color_cost.get())?,
        face_style_cost: price(c.barber_face_cost.get())?,
        eye_color_cost: price(c.barber_eye_cost.get())?,
        ..Default::default()
    })
}
pub fn cost(a: PlayerStyle, b: PlayerStyle, p: BarberPrices) -> Option<u32> {
    if p.change_gender == 0
        && (a.gender != b.gender
            || a.body != b.body
            || a.height != b.height
            || a.skin_color != b.skin_color)
    {
        return None;
    }
    let gender = a.gender != b.gender;
    [
        (gender, p.gender_cost),
        (a.body != b.body || a.height != b.height, p.body_cost),
        (a.skin_color != b.skin_color, p.skin_color_cost),
        (gender || a.hair_style != b.hair_style, p.hair_style_cost),
        (a.hair_color != b.hair_color, p.hair_color_cost),
        (gender || a.face_style != b.face_style, p.face_style_cost),
        (a.eye_color != b.eye_color, p.eye_color_cost),
    ]
    .into_iter()
    .try_fold(0u32, |sum, (changed, price)| {
        if changed {
            sum.checked_add(u32::try_from(price).ok()?)
        } else {
            Some(sum)
        }
    })
}
pub fn open(pkt: Packet, client: &FFClient, state: &mut ShardServerState) -> FFResult<()> {
    let request = *pkt.get::<BarberOpenRequest>()?;
    let pc_id = client.get_player_id()?;
    validate(state, pc_id, request.npc_id)?;
    let prices = prices()?;
    state.get_player_mut(pc_id)?.barber_session = Some(BarberSession {
        npc_id: request.npc_id,
        prices,
    });
    client.send_packet(P_FE2CL_REP_PC_BARBER_OPEN_SUCC, &prices);
    Ok(())
}
pub fn confirm(pkt: Packet, clients: &ClientMap, state: &mut ShardServerState) -> FFResult<()> {
    let request = *pkt.get::<BarberConfirmRequest>()?;
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;
    let mut reply = BarberReply {
        error: 2,
        slots: [-1; 3],
        ..Default::default()
    };
    let result = (|| {
        let session = state
            .get_player(pc_id)?
            .barber_session
            .ok_or_else(|| fail("Barber confirmation without a quote"))?;
        validate(state, pc_id, session.npc_id)?;
        let draft = PlayerStyle::try_from(request.style)?;
        let player = state.get_player_mut(pc_id)?;
        let original = player.style.ok_or_else(|| fail("Missing player style"))?;
        let total =
            cost(original, draft, session.prices).ok_or_else(|| fail("Invalid appearance cost"))?;
        if player.get_taros() < total {
            return Err(fail("Insufficient barber funds"));
        }
        // Plan every destination before touching inventory, appearance or money.
        let empty = (0..SIZEOF_INVEN_SLOT as usize)
            .filter(|slot| {
                player
                    .get_item(ItemLocation::Inven, *slot)
                    .is_ok_and(|item| item.is_none())
            })
            .collect::<Vec<_>>();
        let mut moves = Vec::new();
        for slot in 1..=3 {
            if let Some(item) = *player.get_item(ItemLocation::Equip, slot)? {
                let gender = item.get_stats()?.gender.unwrap_or(0);
                if gender != 0 && gender as i8 != draft.gender {
                    let Some(&dest) = empty.get(moves.len()) else {
                        reply.error = 1;
                        return Err(fail("No inventory room for barber clothes"));
                    };
                    moves.push((slot, dest, item));
                }
            }
        }
        for (slot, dest, item) in moves {
            // All bounds, ownership and trade gates were validated before commit.
            player.set_item(ItemLocation::Equip, slot, None)?;
            player.set_item(ItemLocation::Inven, dest, Some(item))?;
            reply.slots[slot - 1] = dest as i32;
            reply.items[slot - 1] = Some(item).into_proto();
        }
        player.style = Some(draft);
        player.barber_session = None;
        reply.taros = player.set_taros(player.get_taros() - total) as i32;
        reply.error = 0;
        Ok::<_, FFError>(BarberStyleBroadcast {
            pc_id,
            style: player.get_style(),
        })
    })();
    client.send_packet(P_FE2CL_REP_PC_BARBER_CONFIRM, &reply);
    match result {
        Ok(broadcast) => {
            state
                .entity_map
                .for_each_around(EntityID::Player(pc_id), |other| {
                    // Apply equipment before style so the remote candidate uses the final clothes.
                    for i in 0..3 {
                        if reply.slots[i] >= 0 {
                            other.send_packet(
                                P_FE2CL_PC_EQUIP_CHANGE,
                                &sP_FE2CL_PC_EQUIP_CHANGE {
                                    iPC_ID: pc_id,
                                    iEquipSlotNum: (i + 1) as i32,
                                    EquipSlotItem: sItemBase::default(),
                                },
                            );
                        }
                    }
                    other.send_packet(P_FE2CL_PC_CHANGE_STYLE, &broadcast);
                });
            Ok(())
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn price_is_changed_fields_only_and_gender_always_reprices_face_and_hair() {
        let a = PlayerStyle::default();
        let p = BarberPrices {
            change_gender: 1,
            gender_cost: 500,
            body_cost: 100,
            hair_style_cost: 100,
            face_style_cost: 100,
            ..Default::default()
        };
        assert_eq!(cost(a, a, p), Some(0));
        let mut b = a;
        b.gender = 2;
        assert_eq!(cost(a, b, p), Some(700));
        b = a;
        b.height += 1;
        b.body += 1;
        assert_eq!(cost(a, b, p), Some(100));
        let mut forbidden = p;
        forbidden.change_gender = 0;
        assert_eq!(cost(a, b, forbidden), None);
    }
    #[test]
    fn packet_sizes_match_native_0104_contract() {
        assert_eq!(size_of::<BarberOpenRequest>(), 4);
        assert_eq!(size_of::<BarberPrices>(), 288);
        assert_eq!(size_of::<BarberConfirmRequest>(), 76);
        assert_eq!(size_of::<BarberReply>(), 56);
        assert_eq!(size_of::<BarberStyleBroadcast>(), 80);
        assert_eq!(size_of::<NpcMapSnapshotEntry>(), 20);
    }
}

#[cfg(test)]
mod integration_tests;
