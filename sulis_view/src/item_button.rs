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
use std::cell::RefCell;
use std::rc::Rc;

use crate::bonus_text_arg_handler::{
    add_attack_text_args, add_bonus_text_args, add_prereq_text_args,
};
use crate::item_callback_handler::{sell_item_cb, identify_item_cb};
use crate::{ItemActionMenu, MerchantWindow, RootView};
use sulis_core::io::{event, keyboard_event::Key};
use sulis_core::ui::{Callback, Widget, WidgetKind};
use sulis_core::widgets::{Label, TextArea};
use sulis_module::{
    ability,
    item::{format_item_value, format_item_weight},
    Module,
};
use sulis_module::{ItemState, QuickSlot, Slot};
use sulis_state::{inventory::has_proficiency, EntityState, GameState};

enum Kind {
    Prop {
        prop_index: usize,
        item_index: usize,
    },
    Merchant {
        id: String,
        item_index: usize,
    },
    Inventory {
        item_index: usize,
    },
    Equipped {
        player: Rc<RefCell<EntityState>>,
        slot: Slot,
    },
    Quick {
        player: Rc<RefCell<EntityState>>,
        quick: QuickSlot,
    },
}

struct ButtonAction {
    label: String,
    callback: Callback,
    can_left_click: bool,
}

pub struct ItemButton {
    icon: String,
    adjective_icons: Vec<String>,
    quantity: u32,
    kind: Kind,
    actions: Vec<ButtonAction>,
    keyboard_shortcut: Option<Key>,

    item_window: Option<Rc<RefCell<Widget>>>,
}

const ITEM_BUTTON_NAME: &str = "item_button";

impl ItemButton {
    pub fn inventory(
        item: &ItemState,
        quantity: u32,
        item_index: usize,
    ) -> Rc<RefCell<ItemButton>> {
        ItemButton::new(item, quantity, Kind::Inventory { item_index })
    }

    pub fn equipped(
        player: &Rc<RefCell<EntityState>>,
        item: &ItemState,
        slot: Slot,
    ) -> Rc<RefCell<ItemButton>> {
        let player = Rc::clone(player);
        ItemButton::new(item, 1, Kind::Equipped { player, slot })
    }

    pub fn quick(
        player: &Rc<RefCell<EntityState>>,
        quantity: u32,
        item: &ItemState,
        quick: QuickSlot,
    ) -> Rc<RefCell<ItemButton>> {
        let player = Rc::clone(player);
        ItemButton::new(item, quantity, Kind::Quick { player, quick })
    }

    pub fn prop(
        item: &ItemState,
        quantity: u32,
        item_index: usize,
        prop_index: usize,
    ) -> Rc<RefCell<ItemButton>> {
        ItemButton::new(
            item,
            quantity,
            Kind::Prop {
                prop_index,
                item_index,
            },
        )
    }

    pub fn merchant(
        item: &ItemState,
        quantity: u32,
        item_index: usize,
        merchant_id: &str,
    ) -> Rc<RefCell<ItemButton>> {
        ItemButton::new(
            item,
            quantity,
            Kind::Merchant {
                id: merchant_id.to_string(),
                item_index,
            },
        )
    }

    fn new(item: &ItemState, quantity: u32, kind: Kind) -> Rc<RefCell<ItemButton>> {
        let icon = item.icon().id();
        let adjective_icons = item.item.adjective_icons();

        Rc::new(RefCell::new(ItemButton {
            icon,
            adjective_icons,
            quantity,
            kind,
            actions: Vec::new(),
            item_window: None,
            keyboard_shortcut: None,
        }))
    }

    pub fn set_keyboard_shortcut(&mut self, key: Option<Key>) {
        self.keyboard_shortcut = key;
    }

    pub fn fire_left_click_action(&mut self, widget: &Rc<RefCell<Widget>>) {
        let sell_action = self.check_sell_action(widget);
        let cb = sell_action
            .iter()
            .chain(self.actions.iter())
            .find_map(|action| {
                if action.can_left_click {
                    Some(action.callback.clone())
                } else {
                    None
                }
            });
        if let Some(action) = cb {
            action.call(widget, self);
        }
    }

    pub fn add_action(&mut self, name: &str, cb: Callback, can_left_click: bool) {
        let action = ButtonAction {
            label: name.to_string(),
            callback: cb,
            can_left_click,
        };
        self.actions.push(action);
    }

    fn remove_item_window(&mut self) {
        if self.item_window.is_some() {
            self.item_window
                .as_ref()
                .unwrap()
                .borrow_mut()
                .mark_for_removal();
            self.item_window = None;
        }
    }

    fn get_item_state(&self) -> Option<ItemState> {
        let area_state = GameState::area_state();
        let area_state = area_state.borrow();
        match self.kind {
            Kind::Inventory { item_index } => {
                let stash = GameState::party_stash();
                let stash = stash.borrow();
                match stash.items().get(item_index) {
                    None => None,
                    Some(&(_, ref item_state)) => Some(item_state.clone()),
                }
            }
            Kind::Quick { ref player, quick } => {
                let pc = player.borrow();
                match pc.actor.inventory().quick(quick) {
                    None => None,
                    Some(item_state) => Some(item_state.clone()),
                }
            }
            Kind::Equipped { ref player, slot } => {
                let pc = player.borrow();
                match pc.actor.inventory().equipped(slot) {
                    None => None,
                    Some(item_state) => Some(item_state.clone()),
                }
            }
            Kind::Prop {
                prop_index,
                item_index,
            } => {
                if !area_state.props().index_valid(prop_index) {
                    return None;
                }

                match area_state.props().get(prop_index).items() {
                    None => None,
                    Some(ref items) => match items.get(item_index) {
                        None => None,
                        Some(&(_, ref item_state)) => Some(item_state.clone()),
                    },
                }
            }
            Kind::Merchant { ref id, item_index } => {
                let merchant = area_state.get_merchant(id);
                let merchant = match merchant {
                    None => return None,
                    Some(ref merchant) => merchant,
                };

                match merchant.items().get(item_index) {
                    None => None,
                    Some(&(_, ref item_state)) => Some(item_state.clone()),
                }
            }
        }
    }

    fn check_sell_action(&self, widget: &Rc<RefCell<Widget>>) -> Option<ButtonAction> {
        let item_index = match self.kind {
            Kind::Inventory { item_index, .. } => item_index,
            _ => return None,
        };

        // TODO this is a hack putting this here.  but, the state of the merchant
        // window may change after the owing inventory window is opened
        let (root, root_view) = Widget::parent_mut::<RootView>(widget);
        if let Some(window_widget) = root_view.get_merchant_window(&root) {
            let merchant = Widget::kind_mut::<MerchantWindow>(&window_widget).player();

            if self.get_item_state().filter(|i| !i.identified).is_some() {
                Some(ButtonAction {
                    label: "Identify".to_string(),
                    callback: identify_item_cb(merchant, item_index),
                    can_left_click: true,
                })
            } else {
                Some(ButtonAction {
                    label: "Sell".to_string(),
                    callback: sell_item_cb(merchant, item_index),
                    can_left_click: true,
                })
            }
        } else {
            None
        }
    }

    fn add_price_text_arg(
        &self,
        root: &Rc<RefCell<Widget>>,
        item_window: &mut Widget,
        item_state: &ItemState,
    ) {
        let area_state = GameState::area_state();
        let area_state = area_state.borrow();
        match self.kind {
            Kind::Merchant { ref id, .. } => {
                let merchant = area_state.get_merchant(id);
                if let Some(ref merchant) = merchant {
                    let value = merchant.get_buy_price(item_state);
                    item_window
                        .state
                        .add_text_arg("price", &format_item_value(value));
                }
            }
            Kind::Inventory { .. } | Kind::Equipped { .. } => {
                let root_view = Widget::kind_mut::<RootView>(&root);
                let merch_window = match root_view.get_merchant_window(&root) {
                    None => return,
                    Some(window) => window,
                };
                let window = Widget::kind_mut::<MerchantWindow>(&merch_window);
                let merchant = area_state.get_merchant(window.merchant_id());
                if let Some(ref merchant) = merchant {
                    let value = merchant.get_sell_price(item_state);
                    item_window
                        .state
                        .add_text_arg("price", &format_item_value(value));
                }
            }
            _ => (),
        }
    }
}

impl WidgetKind for ItemButton {
    widget_kind!(ITEM_BUTTON_NAME);

    fn on_remove(&mut self, _widget: &Rc<RefCell<Widget>>) {
        self.remove_item_window();
    }

    fn on_add(&mut self, _widget: &Rc<RefCell<Widget>>) -> Vec<Rc<RefCell<Widget>>> {
        let qty_label = Widget::with_theme(Label::empty(), "quantity_label");
        if self.quantity > 1 {
            qty_label
                .borrow_mut()
                .state
                .add_text_arg("quantity", &self.quantity.to_string());
        }
        let icon = Widget::empty("icon");
        icon.borrow_mut().state.add_text_arg("icon", &self.icon);

        let identified = self.get_item_state().filter(|i| i.identified).is_some();
        let adj = Widget::empty("adjectives_pane");
        if identified {
            for icon in self.adjective_icons.iter() {
                let widget = Widget::empty("icon");
                widget.borrow_mut().state.add_text_arg("icon", icon);
                Widget::add_child_to(&adj, widget);
            }
        }

        let key_label = Widget::with_theme(Label::empty(), "key_label");
        if let Some(key) = self.keyboard_shortcut {
            key_label
                .borrow_mut()
                .state
                .add_text_arg("keybinding", &key.short_name());
        }

        vec![icon, adj, qty_label, key_label]
    }

    fn on_mouse_enter(&mut self, widget: &Rc<RefCell<Widget>>) -> bool {
        self.super_on_mouse_enter(widget);

        if self.item_window.is_some() {
            return true;
        }

        let item_state = self.get_item_state();
        let item_state = match item_state {
            None => return true,
            Some(item_state) => item_state,
        };

        let root = Widget::get_root(widget);
        let item_window = Widget::with_theme(TextArea::empty(), "item_window");
        {
            let mut item_window = item_window.borrow_mut();
            item_window.state.disable();
            item_window.state.set_position(
                widget.borrow().state.inner_right(),
                widget.borrow().state.inner_top(),
            );

            if let Some(key) = self.keyboard_shortcut {
                item_window
                    .state
                    .add_text_arg("keybinding", &key.short_name());
            }

            match self.kind {
                Kind::Prop { .. } | Kind::Inventory { .. } | Kind::Merchant { .. } => {
                    let player = GameState::selected();
                    if !player.is_empty() {
                        if !has_proficiency(&item_state, &player[0].borrow().actor.stats) {
                            item_window.state.add_text_arg("prof_not_met", "true");
                        }

                        if !item_state
                            .item
                            .meets_prereqs(&player[0].borrow().actor.actor)
                        {
                            item_window.state.add_text_arg("prereqs_not_met", "true");
                        }

                        if let Some(ref equip) = item_state.item.equippable {
                            if player[0].borrow().actor.actor.race.is_disabled(equip.slot) {
                                item_window
                                    .state
                                    .add_text_arg("slot_disabled_for_race", "true");
                                item_window.state.add_text_arg(
                                    "player_race",
                                    &player[0].borrow().actor.actor.race.name,
                                );
                            }
                        }
                    }
                }
                _ => (),
            }

            item_window
                .state
                .add_text_arg("name", &item_state.get_name());
            let value_str = if item_state.identified {"value"} else {"identification value"};
            item_window
                .state
                .add_text_arg(value_str, &format_item_value(item_state.item.value));

            item_window
                .state
                .add_text_arg("weight", &format_item_weight(item_state.item.weight));
            self.add_price_text_arg(&root, &mut item_window, &item_state);

            if let Some(ref prereqs) = &item_state.item.prereqs {
                add_prereq_text_args(prereqs, &mut item_window.state);
            }

            match &item_state.item.usable {
                None => (),
                Some(usable) => {
                    let state = &mut item_window.state;

                    let ap = Module::rules().to_display_ap(usable.ap as i32);
                    state.add_text_arg("usable_ap", &ap.to_string());
                    if usable.consumable {
                        state.add_text_arg("consumable", "true");
                    }
                    if item_state.identified {
                        match usable.duration {
                            ability::Duration::Rounds(rounds) => {
                                state.add_text_arg("usable_duration", &rounds.to_string())
                            }
                            ability::Duration::Mode => state.add_text_arg("usable_mode", "true"),
                            ability::Duration::Instant => state.add_text_arg("usable_instant", "true"),
                            ability::Duration::Permanent => {
                                state.add_text_arg("usable_permanent", "true")
                            }
                        }
                        state.add_text_arg("usable_description", &usable.short_description);
                    }
                }
            }

            match item_state.item.equippable {
                Some(ref equippable) if item_state.identified => {
                    if let Some(ref attack) = equippable.attack {
                        add_attack_text_args(attack, &mut item_window.state);
                    }
                    add_bonus_text_args(&equippable.bonuses, &mut item_window.state);
                },
                _ => ()
            }
        }
        Widget::add_child_to(&root, Rc::clone(&item_window));
        self.item_window = Some(item_window);

        true
    }

    fn on_mouse_exit(&mut self, widget: &Rc<RefCell<Widget>>) -> bool {
        self.super_on_mouse_exit(widget);

        self.remove_item_window();
        true
    }

    fn on_mouse_release(&mut self, widget: &Rc<RefCell<Widget>>, kind: event::ClickKind) -> bool {
        self.super_on_mouse_release(widget, kind);
        self.remove_item_window();

        match kind {
            event::ClickKind::Primary => {
                self.fire_left_click_action(widget);
            }
            event::ClickKind::Secondary | event::ClickKind::Tertiary => {
                let menu = ItemActionMenu::new();

                let mut at_least_one_action = false;
                if let Some(action) = self.check_sell_action(widget) {
                    menu.borrow_mut().add_action(&action.label, action.callback);
                    at_least_one_action = true;
                }

                for action in self.actions.iter() {
                    menu.borrow_mut()
                        .add_action(&action.label, action.callback.clone());
                    at_least_one_action = true;
                }

                if at_least_one_action {
                    let menu = Widget::with_defaults(menu);
                    menu.borrow_mut().state.set_modal(true);
                    menu.borrow_mut().state.modal_remove_on_click_outside = true;
                    let root = Widget::get_root(widget);
                    Widget::add_child_to(&root, menu);
                }
            }
        }

        true
    }
}
