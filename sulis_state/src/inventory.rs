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
use std::io::Error;
use std::rc::Rc;
use std::slice::Iter;

use sulis_core::image::Image;
use sulis_core::util::invalid_data_error;
use sulis_module::{
    bonus::AttackKindBuilder, Actor, ImageLayer, ItemKind, ItemSaveState, ItemState, Module,
    QuickSlot, Slot, StatList, WeaponStyle,
};

#[derive(Clone)]
pub struct Inventory {
    pub equipped: HashMap<Slot, ItemState>,
    pub quick: HashMap<QuickSlot, ItemState>,
}

impl Inventory {
    pub fn empty() -> Inventory {
        Inventory {
            equipped: HashMap::new(),
            quick: HashMap::new(),
        }
    }

    pub fn load(
        &mut self,
        equipped: Vec<Option<ItemSaveState>>,
        quick: Vec<Option<ItemSaveState>>,
    ) -> Result<(), Error> {
        for (slot_index, slot) in Slot::iter().enumerate() {
            let slot = *slot;

            if slot_index > equipped.len() {
                break;
            }
            let item = match &equipped[slot_index] {
                None => continue,
                Some(item) => item,
            };
            let identified = item.identified;
            let variant = item.variant;
            let item_state = match Module::create_get_item(&item.id, &item.adjectives) {
                None => invalid_data_error(&format!("No item with ID '{}'", item.id)),
                Some(item) => Ok(ItemState::new(item, variant, identified)),
            }?;

            {
                let equippable = match item_state.item.equippable {
                    None => {
                        invalid_data_error(&format!("Item in slot '{:?}' is not equippable", slot))
                    }
                    Some(ref equip) => Ok(equip),
                }?;

                if equippable.slot != slot {
                    let ok = match equippable.alternate_slot {
                        None => false,
                        Some(alt_slot) => alt_slot == slot,
                    };

                    if !ok {
                        return invalid_data_error(&format!(
                            "item in slot '{:?}' invalid equip type",
                            slot
                        ));
                    }
                }
            }

            self.equipped.insert(slot, item_state);
        }

        for (quick_index, quick_slot) in QuickSlot::iter().enumerate() {
            let quick_slot = *quick_slot;

            if quick_index > quick.len() {
                break;
            }
            let item = match &quick[quick_index] {
                None => continue,
                Some(item) => item,
            };

            let variant = item.variant;
            let identified = item.identified;
            let item_state = match Module::create_get_item(&item.id, &item.adjectives) {
                None => invalid_data_error(&format!("No item with ID '{}'", item.id)),
                Some(item) => Ok(ItemState::new(item, variant, identified)),
            }?;

            self.quick.insert(quick_slot, item_state);
        }

        Ok(())
    }

    fn weapon_style_internal(
        &self,
        main: Option<&ItemState>,
        off: Option<&ItemState>,
    ) -> WeaponStyle {
        match off {
            None => (),
            Some(ref item_state) => match item_state.item.kind {
                ItemKind::Armor { .. } => return WeaponStyle::Shielded,
                ItemKind::Weapon { .. } => match main {
                    None => return WeaponStyle::Single,
                    Some(_) => return WeaponStyle::DualWielding,
                },
                ItemKind::Other => (),
            },
        }

        match main {
            None => WeaponStyle::Single,
            Some(ref item_state) => match &item_state.item.equippable {
                Some(ref equippable) => {
                    if let Some(attack_builder) = &equippable.attack {
                        if let AttackKindBuilder::Ranged { .. } = attack_builder.kind {
                            return WeaponStyle::Ranged;
                        }
                    }

                    if let Some(Slot::HeldOff) = equippable.blocks_slot {
                        return WeaponStyle::TwoHanded;
                    }

                    WeaponStyle::Single
                }
                None => unreachable!(),
            },
        }
    }

    pub fn alt_weapon_style(&self) -> WeaponStyle {
        self.weapon_style_internal(
            self.quick(QuickSlot::AltHeldMain),
            self.quick(QuickSlot::AltHeldOff),
        )
    }

    pub fn weapon_style(&self) -> WeaponStyle {
        self.weapon_style_internal(self.equipped(Slot::HeldMain), self.equipped(Slot::HeldOff))
    }

    pub fn quick(&self, slot: QuickSlot) -> Option<&ItemState> {
        self.quick.get(&slot)
    }

    pub fn equipped(&self, slot: Slot) -> Option<&ItemState> {
        self.equipped.get(&slot)
    }

    /// Returns an iterator traversing all equipped items
    /// in this inventory.  This will only include slots that actually
    /// have an item equipped
    pub fn equipped_iter(&self) -> EquippedIterator {
        EquippedIterator {
            slot_iterator: Slot::iter(),
            inventory: &self,
        }
    }

    pub fn swap_weapon_set(&mut self) {
        let cur_main = self.equipped.remove(&Slot::HeldMain);
        let cur_off = self.equipped.remove(&Slot::HeldOff);

        let cur_alt_main = self.quick.remove(&QuickSlot::AltHeldMain);
        let cur_alt_off = self.quick.remove(&QuickSlot::AltHeldOff);

        if let Some(item) = cur_main {
            self.quick.insert(QuickSlot::AltHeldMain, item);
        }

        if let Some(item) = cur_off {
            self.quick.insert(QuickSlot::AltHeldOff, item);
        }

        if let Some(item) = cur_alt_main {
            self.equipped.insert(Slot::HeldMain, item);
        }

        if let Some(item) = cur_alt_off {
            self.equipped.insert(Slot::HeldOff, item);
        }
    }

    #[must_use]
    pub fn clear_quick(&mut self, quick_slot: QuickSlot) -> Option<ItemState> {
        self.quick.remove(&quick_slot)
    }

    /// Returns true if the given item can be set to the given quick slot,
    /// false otherwise
    pub fn can_set_quick(
        &mut self,
        item_state: &ItemState,
        slot: QuickSlot,
        actor: &Rc<Actor>,
    ) -> bool {
        use sulis_module::QuickSlot::*;
        match slot {
            Usable1 | Usable2 | Usable3 | Usable4 => {
                if !item_state.item.meets_prereqs(actor) {
                    return false;
                }
                if item_state.item.usable.is_none() {
                    return false;
                }
            }
            AltHeldMain | AltHeldOff => {
                // TODO validate - this code path is only being used
                // when loading actors from a file
            }
        }

        true
    }

    /// Sets the given item to the quick slot.  The caller must validate that the
    /// item can be set with `can_set_quick` prior to doing this
    #[must_use]
    pub fn set_quick(&mut self, item_state: ItemState, slot: QuickSlot) -> Option<ItemState> {
        self.quick.insert(slot, item_state)
    }

    /// Returns true if the given item can be equipped - use ActorState::can_equip as it
    /// checks all conditions including these
    pub(crate) fn can_equip(
        &self,
        item_state: &ItemState,
        stats: &StatList,
        actor: &Rc<Actor>,
    ) -> bool {
        if !item_state.item.meets_prereqs(actor) {
            return false;
        }
        if !has_proficiency(item_state, stats) {
            return false;
        }

        if item_state.item.equippable.is_none() {
            return false;
        }

        let slot = item_state.item.equippable.as_ref().unwrap().slot;
        if actor.race.is_disabled(slot) {
            return false;
        }

        true
    }

    /// Equips the specified item.  you must verify that the item can be equipped
    /// with `can_equip` first.  Returns a vec of any items that were unequipped as
    /// a result of equipping this item
    #[must_use]
    pub fn equip(&mut self, item_state: ItemState, preferred_slot: Option<Slot>) -> Vec<ItemState> {
        let mut unequipped = Vec::new();

        let (slot, alt_slot, blocked_slot) = match &item_state.item.equippable {
            None => {
                warn!(
                    "Attempted to equip invalid item '{}'.  This is a bug.",
                    item_state.item.id
                );
                return Vec::new();
            }
            Some(ref equippable) => (
                equippable.slot,
                equippable.alternate_slot,
                equippable.blocks_slot,
            ),
        };

        // determine the blocking slots that will need to be removed for both
        // primary and alt, depending on which one is chosen
        let mut to_remove_primary = None;
        let mut to_remove_alt = None;
        for item_state in self.equipped_iter() {
            let equippable = match item_state.item.equippable {
                None => unreachable!(),
                Some(ref equippable) => equippable,
            };

            if let Some(ref blocked_slot) = equippable.blocks_slot {
                if *blocked_slot == slot {
                    to_remove_primary = Some(equippable.slot);
                }

                if let Some(alt_slot) = alt_slot {
                    if *blocked_slot == alt_slot {
                        to_remove_alt = Some(equippable.slot);
                    }
                }
            }
        }

        // now determine whether to go with primary or alt based on how many
        // items need to be removed from each.  prefer primary in a tie
        let slot_to_use = self.determine_slot_to_equip(
            slot,
            alt_slot,
            preferred_slot,
            to_remove_primary,
            to_remove_alt,
        );

        if let Some(slot) = to_remove_primary {
            if let Some(item) = self.unequip(slot) {
                unequipped.push(item);
            }
        }

        if let Some(item) = self.unequip(slot_to_use) {
            unequipped.push(item);
        }

        if let Some(blocked_slot) = blocked_slot {
            if let Some(item) = self.unequip(blocked_slot) {
                unequipped.push(item);
            }
        }

        debug!(
            "Equipping item '{}' into '{:?}'",
            item_state.item.id, slot_to_use
        );
        self.equipped.insert(slot_to_use, item_state);

        debug!("Unequip: {:?}", unequipped);
        unequipped
    }

    fn determine_slot_to_equip(
        &self,
        slot: Slot,
        alt_slot: Option<Slot>,
        preferred_slot: Option<Slot>,
        to_remove_primary: Option<Slot>,
        to_remove_alt: Option<Slot>,
    ) -> Slot {
        let alt_slot = match alt_slot {
            None => return slot,
            Some(slot) => slot,
        };

        if let Some(preferred_slot) = preferred_slot {
            if preferred_slot == slot {
                return slot;
            } else if preferred_slot == alt_slot {
                return alt_slot;
            }
        }

        let mut alt_count = 0;
        let mut primary_count = 0;
        if to_remove_primary.is_some() {
            primary_count += 1;
        }
        if self.equipped.get(&slot).is_some() {
            primary_count += 1;
        }
        if to_remove_alt.is_some() {
            alt_count += 1;
        }
        if self.equipped.get(&alt_slot).is_some() {
            alt_count += 1;
        }

        if alt_count < primary_count {
            alt_slot
        } else {
            slot
        }
    }

    #[must_use]
    pub fn clear_quickslot(&mut self, quick_slot: QuickSlot) -> Option<ItemState> {
        self.quick.remove(&quick_slot)
    }

    #[must_use]
    pub fn unequip(&mut self, slot: Slot) -> Option<ItemState> {
        let result = self.equipped.remove(&slot);
        match &result {
            None => (),
            Some(item_state) => info!("Unequipped '{}' from {:?}", item_state.item.id, slot),
        }
        result
    }

    pub fn get_image_layers(&self) -> HashMap<ImageLayer, Rc<dyn Image>> {
        let mut layers = HashMap::new();

        for (slot, item_state) in self.equipped.iter() {
            let equippable = match item_state.item.equippable {
                None => unreachable!(),
                Some(ref equippable) => equippable,
            };

            let iter = if equippable.slot == *slot {
                // the item is in it's primary slot
                item_state.image_iter()
            } else {
                // the item must be in it's secondary slot
                item_state.alt_image_iter()
            };

            for (layer, ref image) in iter {
                layers.insert(*layer, Rc::clone(image));
            }
        }

        layers
    }
}

pub struct EquippedIterator<'a> {
    slot_iterator: Iter<'static, Slot>,
    inventory: &'a Inventory,
}

impl<'a> Iterator for EquippedIterator<'a> {
    type Item = &'a ItemState;
    fn next(&mut self) -> Option<&'a ItemState> {
        loop {
            let slot = match self.slot_iterator.next() {
                None => return None,
                Some(slot) => slot,
            };

            match self.inventory.equipped.get(&slot) {
                None => (),
                Some(item_state) => return Some(item_state),
            }
        }
    }
}

pub fn has_proficiency(item: &ItemState, stats: &StatList) -> bool {
    match item.item.kind {
        ItemKind::Other => true,
        ItemKind::Armor { kind } => stats.has_armor_proficiency(kind),
        ItemKind::Weapon { kind } => stats.has_weapon_proficiency(kind),
    }
}
