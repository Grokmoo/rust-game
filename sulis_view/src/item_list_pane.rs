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

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use sulis_core::ui::{Callback, Widget, WidgetKind};
use sulis_core::widgets::{Button, ScrollDirection, ScrollPane};
use sulis_module::{Item, ItemState, Module};
use sulis_state::{script::ScriptItemKind, EntityState, GameState};

use crate::{item_callback_handler::*, ItemButton};

pub const NAME: &str = "item_list_pane";

enum Kind {
    Entity,
    Merchant(String),
    Prop(usize),
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Filter {
    All,
    Weapon,
    Armor,
    Accessory,
    Usable,
}

impl Filter {
    fn is_allowed(self, item: &Rc<Item>) -> bool {
        use self::Filter::*;
        match self {
            All => true,
            Weapon => item.is_weapon(),
            Armor => item.is_armor(),
            Accessory => {
                if item.is_weapon() || item.is_armor() {
                    return false;
                }
                item.equippable.is_some()
            }
            Usable => item.usable.is_some(),
        }
    }
}

use self::Filter::*;
const FILTERS_LIST: [Filter; 5] = [All, Weapon, Armor, Accessory, Usable];

pub struct ItemListPane {
    entity: Rc<RefCell<EntityState>>,
    kind: Kind,
    cur_filter: Rc<Cell<Filter>>,
}

impl ItemListPane {
    fn new(
        entity: &Rc<RefCell<EntityState>>,
        kind: Kind,
        cur_filter: &Rc<Cell<Filter>>,
    ) -> Rc<RefCell<ItemListPane>> {
        Rc::new(RefCell::new(ItemListPane {
            entity: Rc::clone(entity),
            kind,
            cur_filter: Rc::clone(cur_filter),
        }))
    }

    pub fn new_entity(
        entity: &Rc<RefCell<EntityState>>,
        cur_filter: &Rc<Cell<Filter>>,
    ) -> Rc<RefCell<ItemListPane>> {
        ItemListPane::new(entity, Kind::Entity, cur_filter)
    }

    pub fn new_prop(
        entity: &Rc<RefCell<EntityState>>,
        prop_index: usize,
        cur_filter: &Rc<Cell<Filter>>,
    ) -> Rc<RefCell<ItemListPane>> {
        ItemListPane::new(entity, Kind::Prop(prop_index), cur_filter)
    }

    pub fn new_merchant(
        entity: &Rc<RefCell<EntityState>>,
        merchant_id: String,
        cur_filter: &Rc<Cell<Filter>>,
    ) -> Rc<RefCell<ItemListPane>> {
        ItemListPane::new(entity, Kind::Merchant(merchant_id), cur_filter)
    }

    fn set_filter(&mut self, filter: Filter, widget: &Rc<RefCell<Widget>>) {
        self.cur_filter.set(filter);
        widget.borrow_mut().invalidate_children();
    }

    fn create_content_merchant(&self, merchant_id: &str) -> Rc<RefCell<Widget>> {
        let area_state = GameState::area_state();
        let area_state = area_state.borrow();
        let merchant = area_state.get_merchant(merchant_id);
        let merchant = match merchant {
            None => return Widget::empty("none"),
            Some(ref merchant) => merchant,
        };

        let scrollpane = ScrollPane::new(ScrollDirection::Vertical);
        let list_content = Widget::with_theme(scrollpane.clone(), "items_list");
        for (index, &(qty, ref item)) in merchant.items().iter().enumerate() {
            if !self.cur_filter.get().is_allowed(&item.item) {
                continue;
            }

            let item_button = ItemButton::merchant(&item, qty, index, merchant_id);
            item_button
                .borrow_mut()
                .add_action("Buy", buy_item_cb(merchant_id, index), true);

            scrollpane
                .borrow()
                .add_to_content(Widget::with_defaults(item_button));
        }

        list_content
    }

    fn create_content_prop(&self, prop_index: usize) -> Rc<RefCell<Widget>> {
        let combat_active = GameState::is_combat_active();

        let area_state = GameState::area_state();
        let area_state = area_state.borrow();
        let prop = area_state.props().get(prop_index);

        let scrollpane = ScrollPane::new(ScrollDirection::Vertical);
        let list_content = Widget::with_theme(scrollpane.clone(), "items_list");
        match prop.items() {
            None => (),
            Some(ref items) => {
                for (index, &(qty, ref item)) in items.iter().enumerate() {
                    if !self.cur_filter.get().is_allowed(&item.item) {
                        continue;
                    }

                    let item_button = ItemButton::prop(&item, qty, index, prop_index);
                    if !combat_active {
                        item_button.borrow_mut().add_action(
                            "Take",
                            take_item_cb(prop_index, index),
                            true,
                        );
                    }
                    scrollpane
                        .borrow()
                        .add_to_content(Widget::with_defaults(item_button));
                }
            }
        }

        list_content
    }

    fn create_content_inventory(&self) -> Rc<RefCell<Widget>> {
        let combat_active = GameState::is_combat_active();

        let actor = &self.entity.borrow().actor;

        let scrollpane = ScrollPane::new(ScrollDirection::Vertical);
        let list_content = Widget::with_theme(scrollpane.clone(), "items_list");

        let stash = GameState::party_stash();
        let stash = stash.borrow();
        for (index, &(quantity, ref item)) in stash.items().iter().enumerate() {
            if !self.cur_filter.get().is_allowed(&item.item) {
                continue;
            }

            let item_but = ItemButton::inventory(&item, quantity, index);

            if let Some(ref usable) = item.item.usable {
                if !combat_active && item.item.meets_prereqs(&actor.actor) {
                    let mut but = item_but.borrow_mut();
                    if usable.use_in_slot {
                        but.add_action(
                            "Add to Use Slot",
                            set_quickslot_cb(&self.entity, index),
                            true,
                        );
                    } else {
                        let kind = ScriptItemKind::Stash(index);
                        but.add_action("Use", use_item_cb(&self.entity, kind), true);
                    }
                }
            }

            if !combat_active && actor.can_equip(&item) {
                item_but
                    .borrow_mut()
                    .add_action("Equip", equip_item_cb(&self.entity, index), true);
            }

            if !combat_active {
                item_but
                    .borrow_mut()
                    .add_action("Drop", drop_item_cb(&self.entity, index), false);
            }

            scrollpane
                .borrow()
                .add_to_content(Widget::with_defaults(item_but));
        }

        list_content
    }
}

impl WidgetKind for ItemListPane {
    widget_kind!(NAME);

    fn on_add(&mut self, _widget: &Rc<RefCell<Widget>>) -> Vec<Rc<RefCell<Widget>>> {
        let mut children = Vec::new();

        let content = match &self.kind {
            Kind::Entity => self.create_content_inventory(),
            Kind::Prop(index) => self.create_content_prop(*index),
            Kind::Merchant(id) => self.create_content_merchant(id),
        };
        children.push(content);

        if let Kind::Entity = &self.kind {
            let coins_item = match Module::item(&Module::rules().coins_item) {
                None => {
                    warn!("Unable to find coins item");
                    return Vec::new();
                }
                Some(item) => item,
            };
            let coins_item_state = ItemState::new(coins_item, None, true);
            let amount =
                GameState::party_coins() as f32 / Module::rules().item_value_display_factor;
            let button = ItemButton::inventory(&coins_item_state, amount as u32, 0);
            let coins_button = Widget::with_theme(button, "coins_button");
            coins_button.borrow_mut().state.set_enabled(false);
            children.push(coins_button);
        }

        for filter in FILTERS_LIST.iter() {
            let filter = *filter;

            let button = Widget::with_theme(
                Button::empty(),
                &format!("filter_{:?}", filter).to_lowercase(),
            );
            button
                .borrow_mut()
                .state
                .add_callback(Callback::new(Rc::new(move |widget, _| {
                    let (parent, pane) = Widget::parent_mut::<ItemListPane>(widget);
                    pane.set_filter(filter, &parent);
                })));
            if filter == self.cur_filter.get() {
                button.borrow_mut().state.set_active(true);
            }
            children.push(button);
        }

        children
    }
}
