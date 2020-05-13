//  This file is part of Sulis, a turn based RPG written in Rust.
//  Copyright 2018 Jared Stephen
//
//  Sulis is free software: you can redistribute it and/or modify
//  it under the terms of the GNU General Public License as published by
//  the Free Software Foundation, either version 3 of the License, or
//  (at your option) any later version.
//
//  Sulis is distributed in the hope that it will be useful,
//  but WITHOUT ANY WARRANTY; without even the implied warranty of
//  MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
//  GNU General Public License for more details.
//
//  You should have received a copy of the GNU General Public License
//  along with Sulis.  If not, see <http://www.gnu.org/licenses/>

use std::collections::HashMap;
use std::rc::Rc;

use crate::rules::{QuickSlot, Slot};
use crate::{Item, ItemState, Module, Race};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct ItemListEntrySaveState {
    pub quantity: u32,
    pub item: ItemSaveState,
}

impl ItemListEntrySaveState {
    pub fn new(quantity: u32, item: &ItemState) -> ItemListEntrySaveState {
        ItemListEntrySaveState {
            quantity,
            item: ItemSaveState::new(item),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct ItemSaveState {
    pub id: String,
    #[serde(default = "bool_true")]
    pub identified: bool,
    #[serde(default)]
    pub adjectives: Vec<String>,

    #[serde(default)]
    pub variant: Option<usize>,
}

impl ItemSaveState {
    pub fn new(item: &ItemState) -> ItemSaveState {
        let adjectives: Vec<_> = item
            .item
            .added_adjectives
            .iter()
            .map(|adj| adj.id.clone())
            .collect();

        ItemSaveState {
            id: item.item.original_id.clone(),
            identified: item.identified,
            adjectives,
            variant: item.variant,
        }
    }
}

fn bool_true() -> bool { true }

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields, default)]
pub struct InventoryBuilder {
    equipped: HashMap<Slot, ItemSaveState>,
    quick: HashMap<QuickSlot, ItemSaveState>,

    #[serde(default)]
    pc_starting_coins: i32,
    #[serde(default)]
    pc_starting_items: Vec<ItemListEntrySaveState>,
}

fn equippable_to(item: &Rc<Item>, item_id: &str, slot: Slot) -> bool {
    match &item.equippable {
        None => {
            warn!(
                "Unequippable item '{}' found in equip slot for inventory",
                item_id
            );
            return false;
        }
        Some(ref equippable) => {
            if equippable.slot != slot {
                let alt = equippable.alternate_slot;
                if alt.is_none() || alt.unwrap() != slot {
                    warn!(
                        "Item '{}' not equippable to slot {:?} in inventory",
                        item_id, slot
                    );
                    return false;
                }
            }
        }
    }
    true
}

impl InventoryBuilder {
    pub fn new(
        equipped: HashMap<Slot, ItemSaveState>,
        quick: HashMap<QuickSlot, ItemSaveState>,
        pc_starting_coins: i32,
        pc_starting_items: Vec<ItemListEntrySaveState>,
    ) -> InventoryBuilder {
        InventoryBuilder {
            equipped,
            quick,
            pc_starting_coins,
            pc_starting_items,
        }
    }

    pub fn remove_invalid_items(&mut self, race: &Rc<Race>) {
        for slot in race.disabled_slots.iter() {
            self.equipped.remove(slot);
        }
    }

    /// Returns the starting currency for players owning this inventory.
    /// This is only relevant for new player characters.
    pub fn pc_starting_coins(&self) -> i32 {
        self.pc_starting_coins
    }

    /// Iterates over the items in this inventory, validating that they exist
    pub fn pc_starting_item_iter<'a>(&'a self) -> impl Iterator<Item = (u32, ItemState)> + 'a {
        self.pc_starting_items.iter().filter_map(|entry| {
            let qty = entry.quantity;
            let item = &entry.item;
            let identified = item.identified;
            match Module::create_get_item(&item.id, &item.adjectives) {
                None => {
                    warn!(
                        "Item '{}' with adjectives '{:?}' not found in inventory",
                        item.id, item.adjectives
                    );
                    None
                }
                Some(item) => {
                    let state = ItemState::new(item, entry.item.variant, identified);
                    Some((qty, state))
                }
            }
        })
    }

    /// Provides an iterator over all the items in this inventory.
    /// Validates as much as possible that items are valid for the specified slots,
    /// but cannot do validations that depend on the actor.
    pub fn equipped_iter<'a>(&'a self) -> impl Iterator<Item = (Slot, ItemState)> + 'a {
        self.equipped.iter().filter_map(|(slot, item_save)| {
            let slot = *slot;
            let item = match Module::create_get_item(&item_save.id, &item_save.adjectives) {
                None => {
                    warn!(
                        "Item '{}' with adjectives '{:?}' not found in equipped",
                        item_save.id, item_save.adjectives
                    );
                    return None;
                }
                Some(item) => item,
            };

            if !equippable_to(&item, &item_save.id, slot) {
                return None;
            }

            Some((slot, ItemState::new(item, item_save.variant, item_save.identified)))
        })
    }

    /// Provides an iterator over all the quick slot item in this defined inventory.
    /// Validates as much as possible that the items are valid for the slots, but cannot do
    /// any validation that also depends on the actor.
    pub fn quick_iter<'a>(&'a self) -> impl Iterator<Item = (QuickSlot, ItemState)> + 'a {
        self.quick.iter().filter_map(|(slot, item_save)| {
            let slot = *slot;
            let item = match Module::create_get_item(&item_save.id, &item_save.adjectives) {
                None => {
                    warn!(
                        "Item '{}' with adjectives '{:?}' not found in quick",
                        item_save.id, item_save.adjectives
                    );
                    return None;
                }
                Some(item) => item,
            };

            use crate::rules::QuickSlot::*;
            match slot {
                AltHeldMain => {
                    if !equippable_to(&item, &item_save.id, Slot::HeldMain) {
                        return None;
                    }
                }
                AltHeldOff => {
                    if !equippable_to(&item, &item_save.id, Slot::HeldOff) {
                        return None;
                    }
                }
                Usable1 | Usable2 | Usable3 | Usable4 => {
                    item.usable.as_ref()?;
                }
            }

            Some((slot, ItemState::new(item, item_save.variant, item_save.identified)))
        })
    }
}

impl Default for InventoryBuilder {
    fn default() -> InventoryBuilder {
        InventoryBuilder {
            equipped: HashMap::new(),
            quick: HashMap::new(),
            pc_starting_items: Vec::new(),
            pc_starting_coins: 0,
        }
    }
}
