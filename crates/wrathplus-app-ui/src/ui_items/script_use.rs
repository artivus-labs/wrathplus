//! Ordered script item uses, including explicit unit contexts.
use super::*;

impl ScriptItemUse<'_, '_> {
    /// `UseInventoryItem(id)`, a doll right-click, sent through [`super::super::send_item_use`], as the
    /// reference's doll click reaches the same `CGItem::Use` a bag click does (`0x4c7af0`): a worn
    /// quest-starter offers its quest. Ids outside 1..=19 are refused.
    pub(crate) fn use_inventory_item_at(
        &mut self,
        script: &mut UiScript,
        id: u32,
        explicit: Option<Option<u64>>,
    ) {
        let Self {
            self_q,
            targeting,
            ladder,
            ui_errors,
            gate,
            ..
        } = self;
        if !(1..=19).contains(&id) {
            debug!("ui_items: UseInventoryItem({id}) out of range — ignored");
            return;
        }
        let slot = (id - 1) as u8;
        let (guid, start_quest, spell_index, use_spell, entry, is_charter) = self_q
            .iter()
            .next()
            .and_then(|store| slot_guid(&store.0, EQUIPMENT_BAG, slot, &ladder.objects))
            .and_then(|guid| {
                let entry = ladder.objects.object(guid)?.object_entry()?;
                let t = ladder.items.template(entry, guid, &ladder.commands)?;
                Some((
                    Some(guid),
                    t.start_quest,
                    t.use_spell_index().unwrap_or(0),
                    t.use_spell.map(|u| u.spell_id),
                    Some(entry),
                    t.flags & wrathplus_protocol::types::ITEM_FLAG_CHARTER != 0,
                ))
            })
            .unwrap_or((None, 0, 0, None, None, false));
        debug!("ui_items: use equipped item, lua slot {id} (wire 255/{slot})");
        super::super::send_item_use(
            super::super::ItemUse {
                guid,
                start_quest,
                bag_index: BAG_PLAYER_INVENTORY,
                slot,
                entry: entry.unwrap_or(0),
                spell_index,
                use_spell,
                on_object: None,
                is_charter,
            },
            &targeting.context_at(explicit),
            ladder,
            script,
            gate,
            false,
            ui_errors,
        );
    }

    /// `UseContainerItem(bag, slot)` outside repair mode: a bank move with the bank open, a sale
    /// with a merchant open (`0x5d881a`'s order), else the equip-vs-use fork and use dispatcher.
    #[cfg(test)]
    pub(crate) fn use_container_item(&mut self, script: &mut UiScript, bag: i64, slot: u32) {
        self.use_container_item_at(script, bag, slot, None);
    }

    pub(crate) fn use_container_item_at(
        &mut self,
        script: &mut UiScript,
        bag: i64,
        slot: u32,
        explicit: Option<Option<u64>>,
    ) {
        let Self {
            self_q,
            merchant,
            bank,
            equip_sound,
            item_text,
            targeting,
            pending_items,
            loot_latch,
            ladder,
            ui_errors,
            gate,
        } = self;
        let slot0 = u8::try_from(slot.saturating_sub(1)).ok();
        let Some((bag_index, wire_slot)) = wire_pos_for(bag, slot) else {
            debug!("ui_items: UseContainerItem({bag}, {slot}) out of range — ignored");
            return;
        };
        // With the bank open, a vault or bank-bag item withdraws (`CMSG_AUTOSTORE_BANK_ITEM`) and
        // any other deposits (`CMSG_AUTOBANK_ITEM`): the resolver's bank flag, set for bags -1
        // and 5..11 (`0x5d7380`), picks the opcode (`0x5d881a`). Doll clicks come through
        // `use_inventory_item`, so worn gear keeps its plain use.
        if bank.is_open() {
            let withdrawing = bag == super::super::BANK_CONTAINER
                || (5..=super::super::StorageSlots::GEOMETRY.last_bag()).contains(&bag);
            if withdrawing {
                debug!("ui_items: withdraw (lua bag {bag} → wire {bag_index}/{wire_slot})");
                let _ = ladder.commands.0.send(ClientCommand::AutoStoreBankItem {
                    bag: bag_index,
                    slot: wire_slot,
                });
            } else {
                debug!("ui_items: deposit (lua bag {bag} → wire {bag_index}/{wire_slot})");
                let _ = ladder.commands.0.send(ClientCommand::AutoBankItem {
                    bag: bag_index,
                    slot: wire_slot,
                });
            }
            return;
        }
        // With a merchant open, the click sells the whole stack (`CMSG_SELL_ITEM`, count 0).
        if let (true, Some(vendor)) = (merchant.is_open(), merchant.vendor) {
            let item_guid = self_q
                .iter()
                .next()
                .and_then(|store| slot_guid(&store.0, bag, slot0.unwrap_or(0), &ladder.objects));
            match item_guid {
                Some(guid) => {
                    debug!("ui_items: sell lua bag {bag} slot {slot} (item {guid:#x})");
                    let _ = ladder.commands.0.send(ClientCommand::SellItem {
                        vendor,
                        item_guid: guid,
                        count: 0,
                    });
                }
                None => debug!("ui_items: sell on empty slot (bag {bag} slot {slot}) — ignored"),
            }
            return;
        }
        // `None` (empty, or the template in flight) falls through to a plain use.
        let clicked = self_q
            .iter()
            .next()
            .and_then(|store| slot_guid(&store.0, bag, slot0.unwrap_or(0), &ladder.objects))
            .and_then(|guid| {
                let obj = ladder.objects.object(guid)?;
                let inst_flags = obj.item_flags().unwrap_or(0);
                // 12340 items carry no `ITEM_FIELD_ITEM_TEXT_ID`.
                let item_text_id = 0;
                let entry = obj.object_entry()?;
                let t = ladder.items.template(entry, guid, &ladder.commands)?;
                Some(Clicked {
                    guid,
                    entry,
                    item_text_id,
                    inventory_type: t.inventory_type,
                    display_info_id: t.display_info_id,
                    start_quest: t.start_quest,
                    spell_index: t.use_spell_index().unwrap_or(0),
                    use_spell: t.use_spell.map(|u| u.spell_id),
                    unwraps_gift: t.unwraps_gift(inst_flags),
                    begins_gift_wrap: t.begins_gift_wrap(inst_flags),
                    opens_loot: t.opens_loot(),
                    page_text: t.page_text,
                    is_charter: t.flags & wrathplus_protocol::types::ITEM_FLAG_CHARTER != 0,
                })
            });

        // The reference's equip-vs-use fork (`0x4fa3b9`): an equippable item auto-equips unless
        // its `StartQuest` (`[rec+0x1a8]`) is set (`0x4fa3bd`). Everything below is the use
        // dispatcher `0x5d8d00`, in its own order.
        if let Some(c) = clicked.filter(|c| c.start_quest == 0 && c.inventory_type != 0) {
            // This path never moves the cursor, so it plays the equip sound itself; a deferred
            // equip plays none, as the reference's sound rides the arm that sends.
            if send_auto_equip(
                script,
                gate,
                &ladder.objects,
                &ladder.items,
                &ladder.commands,
                bag_index,
                wire_slot,
                Some(c.guid),
                false,
            ) {
                equip_sound.write(crate::sound::AutoEquipSound {
                    display_id: c.display_info_id,
                });
            }
            return;
        }
        // A wrapped gift unwraps with `CMSG_OPEN_ITEM` (`0x5d8d92`, emitter `0x5edd60`), first in
        // the dispatcher; vmangos swaps the entry back from `character_gifts`.
        if let Some(c) = clicked.filter(|c| c.unwraps_gift) {
            debug!(
                "ui_items: unwrap gift {:#x} (lua bag {bag} → wire {bag_index}/{wire_slot})",
                c.guid
            );
            let _ = ladder.commands.0.send(ClientCommand::OpenItem {
                bag_index,
                slot: wire_slot,
            });
            return;
        }
        // Wrapping paper (`0x5d8d9d`'s other branch, `0x5edea0`) sends nothing: its slot locks and
        // the cursor becomes mode 2 until a left-click on a container slot sends `CMSG_WRAP_ITEM`.
        if let Some(c) = clicked.filter(|c| c.begins_gift_wrap) {
            debug!(
                "ui_items: arm gift wrap with {:#x} (lua bag {bag} slot {slot})",
                c.guid
            );
            script.arm_gift_wrap(bag, slot);
            return;
        }
        // A quest-starter (`0x5d8dd2`) offers its quest, ahead of the readable and open arms.
        if let Some(c) = clicked.filter(|c| c.start_quest != 0) {
            debug!(
                "ui_items: quest-starter {:#x} offers quest {}",
                c.guid, c.start_quest
            );
            super::super::send_item_use(
                super::super::ItemUse {
                    guid: Some(c.guid),
                    start_quest: c.start_quest,
                    bag_index,
                    slot: wire_slot,
                    entry: c.entry,
                    spell_index: c.spell_index,
                    use_spell: c.use_spell,
                    on_object: None,
                    is_charter: c.is_charter,
                },
                &targeting.context_at(explicit),
                ladder,
                script,
                gate,
                false,
                ui_errors,
            );
            return;
        }
        // A book, a template with `PageText` (`0x5d8e4c`), opens the reader with no packet
        // (`0x4e32e0`). It precedes the open arm, the inverse of the tooltip: an item both
        // readable and lootable shows `<Right Click to Open>` and reads on click.
        if let Some(c) = clicked.filter(|c| c.page_text != 0) {
            if item_text.toggle_closed(c.guid) {
                debug!("ui_items: re-click closes the book {:#x}", c.guid);
            } else {
                debug!(
                    "ui_items: read book {:#x} (page {}, lua bag {bag} slot {slot})",
                    c.guid, c.page_text
                );
                item_text.open_pages(c.guid);
            }
            return;
        }
        // A letter from mail (`ITEM_FIELD_ITEM_TEXT_ID`) opens the reader with no packet; vmangos's
        // `CMSG_READ_ITEM` wants a template `PageText`, so the text comes by item-text query.
        if let Some(c) = clicked.filter(|c| c.item_text_id != 0) {
            if item_text.toggle_closed(c.guid) {
                debug!("ui_items: re-click closes the letter {:#x}", c.guid);
            } else {
                debug!(
                    "ui_items: read letter {:#x} (text {}, lua bag {bag} slot {slot})",
                    c.guid, c.item_text_id
                );
                item_text.open_letter(c.guid);
            }
            return;
        }
        // A lootable template (`0x5d8f7c`, emitter `0x5edc80`) sends `CMSG_OPEN_ITEM`, which the
        // server answers with a loot window on the item (`SpellHandler.cpp:227`). Unlike the
        // tooltip line, the send tests neither the lock id nor the unlocked bit, so a locked box
        // sends and the server's `EQUIP_ERR_ITEM_LOCKED` is the "Item is locked" line.
        if let Some(c) = clicked.filter(|c| c.opens_loot) {
            debug!(
                "ui_items: open item {:#x} (lua bag {bag} → wire {bag_index}/{wire_slot})",
                c.guid
            );
            // The loot latch on the item's guid, before the send (`0x5edcc0`); the admission gate
            // refuses the type-1 `SMSG_LOOT_RESPONSE` without it. No kneel (`0x612710`).
            loot_latch.0 = Some(c.guid);
            // The grey lock, also before the send (lock setter `0x4953e0` at `0x5edcd9`); it holds
            // until the item empties, the server refuses, or `SMSG_LOOT_RELEASE_RESPONSE` unlocks
            // it by guid. The unwrap arm's emitter (`0x5edd60`) takes neither lock nor latch.
            let (guid, count) = self_q
                .iter()
                .next()
                .map(|store| slot_guid_count(Some(store), bag, slot, &ladder.objects))
                .unwrap_or((0, 0));
            pending_items.add([(bag, slot, guid, count)]);
            script.fire_event("ITEM_LOCK_CHANGED", super::super::lock_args(bag, slot));
            let _ = ladder.commands.0.send(ClientCommand::OpenItem {
                bag_index,
                slot: wire_slot,
            });
            return;
        }
        // A plain use (food, potions, the hearthstone); quest-starters took the arm above.
        debug!("ui_items: use item (lua bag {bag} → wire {bag_index}/{wire_slot})");
        super::super::send_item_use(
            super::super::ItemUse {
                guid: clicked.map(|c| c.guid),
                start_quest: 0,
                bag_index,
                slot: wire_slot,
                entry: clicked.map_or(0, |c| c.entry),
                spell_index: clicked.map_or(0, |c| c.spell_index),
                use_spell: clicked.and_then(|c| c.use_spell),
                on_object: None,
                is_charter: clicked.is_some_and(|c| c.is_charter),
            },
            &targeting.context_at(explicit),
            ladder,
            script,
            gate,
            false,
            ui_errors,
        );
    }
}
