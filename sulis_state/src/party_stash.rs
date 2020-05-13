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

use crate::{ChangeListenerList, GameState, ItemList};
use sulis_module::{ItemListEntrySaveState, ItemState, Module};

pub struct PartyStash {
    items: ItemList,
    coins_id: String,
    pub listeners: ChangeListenerList<PartyStash>,
}

impl PartyStash {
    pub(crate) fn new(items: ItemList) -> PartyStash {
        let coins_id = Module::rules().coins_item.to_string();
        PartyStash {
            items,
            coins_id,
            listeners: ChangeListenerList::default(),
        }
    }

    pub(crate) fn save(&self) -> Vec<ItemListEntrySaveState> {
        self.items
            .iter()
            .map(|(q, ref i)| ItemListEntrySaveState::new(*q, &i))
            .collect()
    }

    pub fn items(&self) -> &ItemList {
        &self.items
    }

    pub fn add_item(&mut self, quantity: u32, item_state: ItemState) -> Option<usize> {
        if quantity == 0 {
            return None;
        }

        if item_state.item.id == self.coins_id {
            let qty = quantity as i32 * Module::rules().item_value_display_factor as i32;
            GameState::add_party_coins(qty);
            return None;
        }

        let index = self.items.add_quantity(quantity, item_state);

        self.listeners.notify(&self);

        Some(index)
    }

    /// Returns whether or not this stash has at least one item
    /// with the specified ID
    pub fn has_item(&self, id: &str) -> bool {
        for (_, ref item) in self.items.iter() {
            if item.item.id == id {
                return true;
            }
        }

        false
    }

    #[must_use]
    /// Removes one item from the specified index.  returns it if there
    /// was an item to remove
    pub fn remove_item(&mut self, index: usize) -> Option<ItemState> {
        let result = self.items.remove(index);

        self.listeners.notify(&self);

        result
    }

    pub fn identify(&mut self, index: usize) -> Option<&ItemState> {
        self.items.identify(index)
    }

    /// Takes all items out of the specified prop and into this stash
    pub fn take_all(&mut self, prop_index: usize) {
        let area_state = GameState::area_state();
        let mut area_state = area_state.borrow_mut();
        let prop_state = area_state.props_mut().get_mut(prop_index);

        let num_items = match prop_state.items() {
            None => return,
            Some(ref items) => items.len(),
        };

        if num_items > 0 {
            let mut i = num_items - 1;
            loop {
                if let Some((qty, item_state)) = prop_state.remove_all_at(i) {
                    self.add_item(qty, item_state);
                }

                if i == 0 {
                    break;
                }

                i -= 1;
            }
            self.listeners.notify(&self);
        }
    }

    /// takes one item-index out of the specified prop and into this stash
    pub fn take(&mut self, prop_index: usize, item_index: usize) {
        let area_state = GameState::area_state();
        let mut area_state = area_state.borrow_mut();
        let prop_state = area_state.props_mut().get_mut(prop_index);

        if let Some((qty, item_state)) = prop_state.remove_all_at(item_index) {
            self.add_item(qty, item_state);
        }

        self.listeners.notify(&self);
    }
}
