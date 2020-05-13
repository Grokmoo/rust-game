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

use std::ops::Index;
use std::slice::Iter;

use sulis_module::ItemState;

#[derive(Clone, Debug)]
pub struct ItemList {
    items: Vec<(u32, ItemState)>,
}

impl Index<usize> for ItemList {
    type Output = (u32, ItemState);
    fn index(&self, index: usize) -> &(u32, ItemState) {
        &self.items[index]
    }
}

impl Default for ItemList {
    fn default() -> ItemList {
        ItemList { items: Vec::new() }
    }
}

impl ItemList {
    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn iter(&self) -> Iter<(u32, ItemState)> {
        self.items.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.items.len() == 0
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn get(&self, index: usize) -> Option<&(u32, ItemState)> {
        self.items.get(index)
    }

    pub fn get_quantity(&self, item: &ItemState) -> u32 {
        for &(qty, ref item_in_list) in self.items.iter() {
            if item == item_in_list {
                return qty;
            }
        }

        0
    }

    pub fn find_index(&self, state: &ItemState) -> Option<usize> {
        for (i, &(_, ref item)) in self.items.iter().enumerate() {
            if item == state {
                return Some(i);
            }
        }

        None
    }

    /// Adds the specified count of this item, and returns the index
    /// the item was placed at
    pub fn add_quantity(&mut self, qty: u32, item_state: ItemState) -> usize {
        match self.find_index(&item_state) {
            Some(index) => {
                self.items[index].0 += qty;
                index
            }
            None => {
                self.items.push((qty, item_state));
                self.items.len() - 1
            }
        }
    }

    /// Adds one count of the specified item, and returns the index that
    /// the item was placed at
    pub fn add(&mut self, item_state: ItemState) -> usize {
        self.add_quantity(1, item_state)
    }

    /// Removes the entire quantity of items at the specified index and returns it
    pub fn remove_all_at(&mut self, index: usize) -> Option<(u32, ItemState)> {
        if index >= self.items.len() {
            return None;
        }

        Some(self.items.remove(index))
    }

    pub fn identify(&mut self, index: usize) -> Option<&ItemState> {
        if index >= self.items.len() {
            return None;
        }
        self.items[index].1.identified = true;
        Some(&self.items[index].1)
    }

    /// Remove an item from the list at the specified index and returns it.
    /// Only removes one count, so the item may still exist if there is more
    /// than one
    pub fn remove(&mut self, index: usize) -> Option<ItemState> {
        if index >= self.items.len() {
            return None;
        }

        if self.items[index].0 == 1 {
            let (_, item_state) = self.items.remove(index);

            Some(item_state)
        } else {
            self.items[index].0 -= 1;
            Some(self.items[index].1.clone())
        }
    }
}
