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

use std::io::Error;
use std::rc::Rc;

use sulis_core::util::invalid_data_error;
use sulis_module::{ItemState, LootList, Module, Time};

use crate::{save_state::MerchantSaveState, ChangeListenerList, GameState, ItemList};

pub struct MerchantState {
    pub id: String,
    pub buy_frac: f32,
    pub sell_frac: f32,
    pub identify_fraction: f32,
    pub listeners: ChangeListenerList<MerchantState>,
    items: ItemList,

    pub loot_list_id: Option<String>,
    pub refresh_rate_millis: usize,
    pub last_refresh_millis: usize,
}

impl MerchantState {
    pub fn load(save: MerchantSaveState) -> Result<MerchantState, Error> {
        let mut items = ItemList::default();
        for item_save in save.items {
            let item = item_save.item;
            let variant = item.variant;
            let identified = item.identified;
            let item = match Module::create_get_item(&item.id, &item.adjectives) {
                None => invalid_data_error(&format!("No item with ID '{}'", item.id)),
                Some(item) => Ok(item),
            }?;

            items.add_quantity(item_save.quantity, ItemState::new(item, variant, identified));
        }

        Ok(MerchantState {
            id: save.id,
            loot_list_id: save.loot_list_id,
            buy_frac: save.buy_frac,
            sell_frac: save.sell_frac,
            identify_fraction: 0.2,
            listeners: ChangeListenerList::default(),
            items,
            refresh_rate_millis: save.refresh_rate_millis,
            last_refresh_millis: save.last_refresh_millis,
        })
    }

    pub fn new(
        id: &str,
        loot_list: &Rc<LootList>,
        buy_frac: f32,
        sell_frac: f32,
        refresh_time: Time,
    ) -> MerchantState {
        let mgr = GameState::turn_manager();
        let last_refresh_millis = mgr.borrow().total_elapsed_millis();
        let refresh_rate_millis = Module::rules().compute_millis(refresh_time);

        let mut items = ItemList::default();

        for (qty, item) in loot_list.generate() {
            items.add_quantity(qty, item);
        }

        MerchantState {
            id: id.to_string(),
            loot_list_id: Some(loot_list.id.to_string()),
            buy_frac,
            sell_frac,
            identify_fraction: 0.2,
            items,
            listeners: ChangeListenerList::default(),
            last_refresh_millis,
            refresh_rate_millis,
        }
    }

    pub fn check_refresh(&mut self) {
        if self.refresh_rate_millis == 0 {
            return;
        }

        let mgr = GameState::turn_manager();
        let cur_millis = mgr.borrow().total_elapsed_millis();
        info!(
            "Check merchant '{}' refresh with current millis: {}, \
             refresh rate: {}, last_refresh: {}",
            self.id, cur_millis, self.refresh_rate_millis, self.last_refresh_millis
        );
        if cur_millis < self.last_refresh_millis + self.refresh_rate_millis {
            return;
        }

        self.last_refresh_millis = cur_millis;

        let loot_list_id = match self.loot_list_id {
            None => return,
            Some(ref id) => id,
        };

        let loot_list = match Module::loot_list(loot_list_id) {
            None => {
                warn!(
                    "Invalid loot list '{}' saved in merchant state",
                    loot_list_id
                );
                return;
            }
            Some(list) => list,
        };

        self.items.clear();
        for (qty, item) in loot_list.generate() {
            self.items.add_quantity(qty, item);
        }
    }

    pub fn get_buy_price(&self, item_state: &ItemState) -> i32 {
        ((item_state.item.value as f32) * self.buy_frac).ceil() as i32
    }

    pub fn get_sell_price(&self, item_state: &ItemState) -> i32 {
        let value = if item_state.identified {
            item_state.item.value as f32
        } else {
            (1.0 - self.identify_fraction) * item_state.item.value.abs() as f32
        };
        (value * self.sell_frac).floor() as i32
    }

    pub fn add(&mut self, item_state: ItemState) {
        self.items.add(item_state);

        self.listeners.notify(&self);
    }

    /// removes one copy of the item at the specified index
    pub fn remove(&mut self, index: usize) -> Option<ItemState> {
        let result = self.items.remove(index);

        if result.is_some() {
            self.listeners.notify(&self);
        }

        result
    }

    pub fn items(&self) -> &ItemList {
        &self.items
    }
}
