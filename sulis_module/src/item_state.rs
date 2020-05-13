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

use std::collections::hash_map::Iter;
use std::rc::Rc;

use crate::{ImageLayer, Item, Module};
use sulis_core::image::Image;

#[derive(Debug, Clone)]
pub struct ItemState {
    pub item: Rc<Item>,
    pub variant: Option<usize>,
    pub identified: bool,
}

impl PartialEq for ItemState {
    fn eq(&self, other: &ItemState) -> bool {
        Rc::ptr_eq(&self.item, &other.item) && self.variant == other.variant
    }
}

impl ItemState {
    pub fn new(item: Rc<Item>, variant: Option<usize>, identified: bool) -> ItemState {
        match variant {
            None => ItemState {
                item,
                variant: None,
                identified,
            },
            Some(idx) => {
                if idx >= item.num_variants() {
                    warn!("Invalid variant {} for item {}", idx, item.id);
                    ItemState {
                        item,
                        variant: None,
                        identified,
                    }
                } else {
                    ItemState { item, variant, identified }
                }
            }
        }
    }

    pub fn from(id: &str) -> Option<ItemState> {
        match Module::item(id) {
            None => None,
            Some(item) => Some(ItemState::new(item, None, true)),
        }
    }

    pub fn image_iter(&self) -> Iter<ImageLayer, Rc<dyn Image>> {
        self.item.image_iter(self.variant)
    }

    pub fn alt_image_iter(&self) -> Iter<ImageLayer, Rc<dyn Image>> {
        self.item.alt_image_iter(self.variant)
    }

    pub fn icon(&self) -> Rc<dyn Image> {
        self.item.icon(self.variant)
    }
    pub fn get_name(&self) -> String {
        match &self.item.unidentified_name {
            Some(unidentified_name) if !self.identified => unidentified_name.to_string(),
            _ => self.item.name.to_string()
        }
    }
}
