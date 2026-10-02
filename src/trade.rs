use crate::{
    defines::*,
    entity::Player,
    enums::*,
    error::{panic_log, FFError, FFResult, Severity},
    item::Item,
    net::packet::*,
};

#[derive(Default, Clone, Copy)]
struct TradeItem {
    pub inven_slot_num: usize,
    pub quantity: u16,
    pub identity: Option<Item>,
}
#[derive(Default, Clone, Copy)]
struct TradeOffer {
    taros: u32,
    items: [Option<TradeItem>; 5],
    confirmed: bool,
}
impl TradeOffer {
    fn get_count(&self, inven_slot_num: usize) -> u32 {
        let mut quantity = 0u32;
        for trade_item in self.items.iter().flatten() {
            if trade_item.inven_slot_num == inven_slot_num {
                quantity += u32::from(trade_item.quantity);
            }
        }
        quantity
    }

    fn add_item(
        &mut self,
        trade_slot_num: usize,
        inven_slot_num: usize,
        quantity: u16,
    ) -> FFResult<u16> {
        if trade_slot_num >= self.items.len() {
            return Err(FFError::build(
                Severity::Warning,
                format!("Trade slot number {} out of range", trade_slot_num),
            ));
        }

        self.items[trade_slot_num] = Some(TradeItem {
            inven_slot_num,
            quantity,
            identity: None,
        });

        u16::try_from(self.get_count(inven_slot_num))
            .map_err(|_| trade_error("Trade quantity overflow"))
    }

    fn remove_item(&mut self, trade_slot_num: usize) -> FFResult<(u16, usize)> {
        if trade_slot_num >= self.items.len() {
            return Err(FFError::build(
                Severity::Warning,
                format!("Trade slot number {} out of range", trade_slot_num),
            ));
        }

        if self.items[trade_slot_num].is_none() {
            return Err(FFError::build(
                Severity::Warning,
                format!("Nothing in trade slot {}", trade_slot_num),
            ));
        }

        let removed_item = self.items[trade_slot_num].take().unwrap();
        Ok((
            u16::try_from(self.get_count(removed_item.inven_slot_num))
                .map_err(|_| trade_error("Trade quantity overflow"))?,
            removed_item.inven_slot_num,
        ))
    }
}
pub struct TradeContext {
    from_pc_id: i32,
    from_offer: TradeOffer,
    to_pc_id: i32,
    to_offer: TradeOffer,
}
impl TradeContext {
    pub fn new(from_pc_id: i32, to_pc_id: i32) -> Self {
        Self {
            from_pc_id,
            from_offer: TradeOffer::default(),
            to_pc_id,
            to_offer: TradeOffer::default(),
        }
    }

    pub fn get_id_from(&self) -> i32 {
        self.from_pc_id
    }

    pub fn get_id_to(&self) -> i32 {
        self.to_pc_id
    }

    pub fn get_other_id(&self, pc_id: i32) -> i32 {
        if self.from_pc_id != pc_id {
            return self.from_pc_id;
        }

        if self.to_pc_id != pc_id {
            return self.to_pc_id;
        }

        panic_log("Bad trade state");
    }

    fn get_offer_mut(&mut self, pc_id: i32) -> FFResult<&mut TradeOffer> {
        if pc_id == self.from_pc_id {
            return Ok(&mut self.from_offer);
        }

        if pc_id == self.to_pc_id {
            return Ok(&mut self.to_offer);
        }

        Err(FFError::build(
            Severity::Warning,
            format!("Player {} not in trade", pc_id),
        ))
    }

    fn invalidate_confirmations(&mut self) {
        self.from_offer.confirmed = false;
        self.to_offer.confirmed = false;
    }

    pub fn set_taros(&mut self, pc_id: i32, taros: u32) -> FFResult<()> {
        self.get_offer_mut(pc_id)?.taros = taros;
        self.invalidate_confirmations();
        Ok(())
    }

    pub fn add_item(
        &mut self,
        pc_id: i32,
        trade_slot_num: usize,
        inven_slot_num: usize,
        quantity: u16,
    ) -> FFResult<u16> {
        self.add_item_checked(pc_id, trade_slot_num, inven_slot_num, quantity, u16::MAX)
    }

    pub fn add_item_checked(
        &mut self,
        pc_id: i32,
        trade_slot_num: usize,
        inven_slot_num: usize,
        quantity: u16,
        available: u16,
    ) -> FFResult<u16> {
        let offer = self.get_offer_mut(pc_id)?;
        let mut candidate = *offer;
        if quantity == 0 {
            return Err(trade_error("Empty trade quantity"));
        }
        let count = candidate.add_item(trade_slot_num, inven_slot_num, quantity)?;
        if count > available {
            return Err(trade_error("Insufficient trade quantity"));
        }
        *offer = candidate;
        self.invalidate_confirmations();
        Ok(count)
    }

    pub fn add_inventory_item(
        &mut self,
        pc_id: i32,
        trade_slot: usize,
        inventory_slot: usize,
        quantity: u16,
        item: Item,
    ) -> FFResult<u16> {
        let count =
            self.add_item_checked(pc_id, trade_slot, inventory_slot, quantity, item.quantity)?;
        self.get_offer_mut(pc_id)?.items[trade_slot]
            .as_mut()
            .unwrap()
            .identity = Some(item);
        Ok(count)
    }

    pub fn remove_item(&mut self, pc_id: i32, trade_slot_num: usize) -> FFResult<(u16, usize)> {
        let result = self.get_offer_mut(pc_id)?.remove_item(trade_slot_num)?;
        self.invalidate_confirmations();
        Ok(result)
    }

    fn is_ready(&self) -> bool {
        self.from_offer.confirmed && self.to_offer.confirmed
    }

    pub fn lock_in(&mut self, pc_id: i32) -> FFResult<bool> {
        let offer = self.get_offer_mut(pc_id)?;
        offer.confirmed = true;
        Ok(self.is_ready())
    }

    pub fn resolve(
        self,
        players: (&mut Player, &mut Player),
    ) -> FFResult<(
        [sItemTrade; SIZEOF_TRADE_SLOT as usize],
        [sItemTrade; SIZEOF_TRADE_SLOT as usize],
    )> {
        if self.from_pc_id == self.to_pc_id {
            return Err(trade_error("Self trade"));
        }
        let first_is_from = players.0.get_player_id() == self.from_pc_id;
        let (from, to) = if players.0.get_player_id() == self.from_pc_id
            && players.1.get_player_id() == self.to_pc_id
        {
            (players.0, players.1)
        } else if players.1.get_player_id() == self.from_pc_id
            && players.0.get_player_id() == self.to_pc_id
        {
            (players.1, players.0)
        } else {
            return Err(trade_error("Trade participant mismatch"));
        };
        let mut staged_from = from.clone();
        let mut staged_to = to.clone();
        let from_cash = staged_from
            .get_taros()
            .checked_sub(self.from_offer.taros)
            .and_then(|v| v.checked_add(self.to_offer.taros))
            .filter(|v| *v <= PC_CANDY_MAX)
            .ok_or_else(|| trade_error("Invalid trade wallet"))?;
        let to_cash = staged_to
            .get_taros()
            .checked_sub(self.to_offer.taros)
            .and_then(|v| v.checked_add(self.from_offer.taros))
            .filter(|v| *v <= PC_CANDY_MAX)
            .ok_or_else(|| trade_error("Invalid trade wallet"))?;
        fn extract(offer: &TradeOffer, player: &mut Player) -> FFResult<Vec<(usize, Item)>> {
            let mut items = Vec::new();
            for (offer_slot, item) in offer.items.iter().enumerate() {
                let Some(item) = item else {
                    continue;
                };
                let slot = player.get_item_mut(ItemLocation::Inven, item.inven_slot_num)?;
                if let Some(expected) = item.identity {
                    if !slot
                        .as_ref()
                        .is_some_and(|stack| stack.id == expected.id && stack.ty == expected.ty)
                    {
                        return Err(trade_error("Trade item identity changed"));
                    }
                }
                if !slot
                    .as_ref()
                    .is_some_and(|stack| stack.quantity >= item.quantity && item.quantity > 0)
                {
                    return Err(trade_error("Trade inventory changed"));
                }
                items.push((
                    offer_slot,
                    Item::split_items(slot, item.quantity)
                        .ok_or_else(|| trade_error("Empty trade item"))?,
                ));
            }
            Ok(items)
        }
        fn deposit(
            items: Vec<(usize, Item)>,
            player: &mut Player,
        ) -> FFResult<[sItemTrade; SIZEOF_TRADE_SLOT as usize]> {
            let mut result = [sItemTrade {
                iType: 0,
                iID: 0,
                iOpt: 0,
                iInvenNum: 0,
                iSlotNum: 0,
            }; SIZEOF_TRADE_SLOT as usize];
            for (index, (offer_slot, item)) in items.into_iter().enumerate() {
                let slot = player.find_free_slot(ItemLocation::Inven)?;
                player.set_item(ItemLocation::Inven, slot, Some(item))?;
                result[index] = sItemTrade {
                    iType: item.ty as i16,
                    iID: item.id,
                    iOpt: item.quantity as i32,
                    iInvenNum: slot as i32,
                    iSlotNum: offer_slot as i32,
                };
            }
            Ok(result)
        }
        let outgoing_from = extract(&self.from_offer, &mut staged_from)?;
        let outgoing_to = extract(&self.to_offer, &mut staged_to)?;
        let received_from = deposit(outgoing_to, &mut staged_from)?;
        let received_to = deposit(outgoing_from, &mut staged_to)?;
        staged_from.set_taros(from_cash);
        staged_to.set_taros(to_cash);
        *from = staged_from;
        *to = staged_to;
        // Handler order can start with either participant; return in that order.
        if first_is_from {
            Ok((received_from, received_to))
        } else {
            Ok((received_to, received_from))
        }
    }
}

fn trade_error(message: &str) -> FFError {
    FFError::build(Severity::Warning, message.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn players() -> (Player, Player) {
        crate::tabledata::tdata_init().unwrap();
        let mut a = Player::new(1, 0);
        a.set_player_id(1);
        a.set_taros(100);
        let mut b = Player::new(2, 0);
        b.set_player_id(2);
        b.set_taros(100);
        (a, b)
    }
    fn item(id: i16) -> Item {
        Item::new(ItemType::General, id)
    }
    #[test]
    fn changing_either_offer_requires_both_players_to_confirm_again() {
        let mut trade = TradeContext::new(1, 2);
        assert!(!trade.lock_in(1).unwrap());
        trade.set_taros(2, 10).unwrap();
        assert!(!trade.lock_in(2).unwrap());
        assert!(trade.lock_in(1).unwrap());
        trade.add_item_checked(1, 4, 0, 1, 1).unwrap();
        assert!(!trade.lock_in(2).unwrap());
        assert!(trade.lock_in(1).unwrap());
    }
    #[test]
    fn full_inventory_swap_and_reversed_caller_order_preserve_slots_and_items() {
        let (mut a, mut b) = players();
        for slot in 0..SIZEOF_INVEN_SLOT as usize {
            a.set_item(ItemLocation::Inven, slot, Some(item(1)))
                .unwrap();
            b.set_item(ItemLocation::Inven, slot, Some(item(2)))
                .unwrap();
        }
        let mut trade = TradeContext::new(1, 2);
        trade.add_item_checked(1, 3, 0, 1, 1).unwrap();
        trade.add_item_checked(2, 4, 0, 1, 1).unwrap();
        trade.set_taros(1, 10).unwrap();
        let (b_received, a_received) = trade.resolve((&mut b, &mut a)).unwrap();
        assert_eq!(b_received[0].iID, 1);
        assert_eq!(b_received[0].iSlotNum, 3);
        assert_eq!(a_received[0].iID, 2);
        assert_eq!(a_received[0].iSlotNum, 4);
        assert_eq!(a.get_taros(), 90);
        assert_eq!(b.get_taros(), 110);
        assert_eq!(a.get_item(ItemLocation::Inven, 0).unwrap().unwrap().id, 2);
        assert_eq!(b.get_item(ItemLocation::Inven, 0).unwrap().unwrap().id, 1);
    }
    #[test]
    fn missing_item_insufficient_wallet_and_capacity_do_not_commit_partial_transfers() {
        for mode in 0..3 {
            let (mut a, mut b) = players();
            a.set_item(ItemLocation::Inven, 0, Some(item(1))).unwrap();
            if mode == 2 {
                for slot in 0..SIZEOF_INVEN_SLOT as usize {
                    b.set_item(ItemLocation::Inven, slot, Some(item(2)))
                        .unwrap();
                }
            }
            let mut trade = TradeContext::new(1, 2);
            trade
                .add_item_checked(1, 0, if mode == 0 { 1 } else { 0 }, 1, 1)
                .unwrap();
            trade
                .set_taros(1, if mode == 1 { 101 } else { 10 })
                .unwrap();
            assert!(trade.resolve((&mut a, &mut b)).is_err());
            assert_eq!(a.get_taros(), 100);
            assert_eq!(b.get_taros(), 100);
            assert_eq!(*a.get_item(ItemLocation::Inven, 0).unwrap(), Some(item(1)));
        }
    }
    #[test]
    fn aggregate_quantity_and_overflow_are_rejected_atomically() {
        let mut trade = TradeContext::new(1, 2);
        trade.add_item_checked(1, 0, 0, u16::MAX, u16::MAX).unwrap();
        assert!(trade.add_item_checked(1, 1, 0, 1, u16::MAX).is_err());
        assert_eq!(trade.remove_item(1, 0).unwrap(), (0, 0));
    }
    #[tokio::test]
    async fn agreed_trade_survives_atomic_database_save_and_reload() {
        use crate::database::{open_test_sqlite, DbImpl};
        let (mut a, mut b) = players();
        let directory = "target/bug039";
        std::fs::create_dir_all(directory).unwrap();
        let db = open_test_sqlite(&format!("{directory}/trade-{}.db", uuid::Uuid::new_v4())).await;
        let account_a = db.create_account("trade_from", "isolated-test-hash").await.unwrap();
        let account_b = db.create_account("trade_to", "isolated-test-hash").await.unwrap();
        a.first_name = "Trade".to_owned(); a.last_name = "From".to_owned();
        b.first_name = "Trade".to_owned(); b.last_name = "To".to_owned();
        a.set_item(ItemLocation::Inven, 0, Some(item(1))).unwrap();
        b.set_item(ItemLocation::Inven, 0, Some(item(2))).unwrap();
        db.init_player(account_a.id, &a).await.unwrap();
        db.init_player(account_b.id, &b).await.unwrap();
        let mut trade = TradeContext::new(1, 2);
        trade.add_inventory_item(1, 2, 0, 1, item(1)).unwrap();
        trade.add_inventory_item(2, 3, 0, 1, item(2)).unwrap();
        trade.set_taros(1, 25).unwrap();
        assert!(!trade.lock_in(1).unwrap()); assert!(trade.lock_in(2).unwrap());
        trade.resolve((&mut a, &mut b)).unwrap();
        db.save_players(&[&a, &b]).await.unwrap();
        let loaded_a = db.load_player(account_a.id, 1).await.unwrap().unwrap();
        let loaded_b = db.load_player(account_b.id, 2).await.unwrap().unwrap();
        assert_eq!(loaded_a.get_taros(), 75); assert_eq!(loaded_b.get_taros(), 125);
        assert_eq!(*loaded_a.get_item(ItemLocation::Inven, 0).unwrap(), Some(item(2)));
        assert_eq!(*loaded_b.get_item(ItemLocation::Inven, 0).unwrap(), Some(item(1)));
    }

}
