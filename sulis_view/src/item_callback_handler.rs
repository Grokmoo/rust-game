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

use std::cell::RefCell;
use std::rc::Rc;

use sulis_core::ui::{Callback, Widget};
use sulis_module::{ItemState, QuickSlot, Slot};
use sulis_state::{script::{ScriptCallback, ScriptItemKind}, EntityState, GameState, Script};

use crate::{MerchantWindow, PropWindow, RootView};

pub fn clear_quickslot_cb(entity: &Rc<RefCell<EntityState>>, slot: QuickSlot) -> Callback {
    let entity = Rc::clone(entity);
    Callback::new(Rc::new(move |_, _| {
        let item = {
            let actor = &mut entity.borrow_mut().actor;
            actor.clear_quick(slot)
        };
        if let Some(item) = item {
            let stash = GameState::party_stash();
            stash.borrow_mut().add_item(1, item);
        }
    }))
}

pub fn set_quickslot_cb(entity: &Rc<RefCell<EntityState>>, index: usize) -> Callback {
    let entity = Rc::clone(entity);
    Callback::new(Rc::new(move |_, _| {
        let stash = GameState::party_stash();
        let item = match stash.borrow_mut().remove_item(index) {
            None => return,
            Some(item) => item,
        };

        let to_add = {
            let actor = &mut entity.borrow_mut().actor;
            for slot in QuickSlot::usable_iter() {
                if actor.inventory().quick(*slot).is_none() {
                    let _ = actor.set_quick(item, *slot); // we know there is no item here
                    return;
                }
            }

            actor.set_quick(item, QuickSlot::Usable1)
        };

        if let Some(item) = to_add {
            stash.borrow_mut().add_item(1, item);
        }
    }))
}

pub fn use_item_cb(entity: &Rc<RefCell<EntityState>>, kind: ScriptItemKind) -> Callback {
    let entity = Rc::clone(entity);
    Callback::new(Rc::new(move |_, _| {
        if let ScriptItemKind::Quick(slot) = kind {
            if !entity.borrow().actor.can_use_quick(slot) {
                return;
            }
        }
        Script::item_on_activate(&entity, "on_activate".to_string(), kind.clone());
    }))
}

pub fn take_item_cb(prop_index: usize, index: usize) -> Callback {
    Callback::with(Box::new(move || {
        let stash = GameState::party_stash();
        stash.borrow_mut().take(prop_index, index);
    }))
}

pub fn equip_item_cb(entity: &Rc<RefCell<EntityState>>, index: usize) -> Callback {
    let entity = Rc::clone(entity);
    Callback::with(Box::new(move || {
        let stash = GameState::party_stash();
        let item = match stash.borrow_mut().remove_item(index) {
            None => return,
            Some(item) => item,
        };

        let slot = item.item.equippable.as_ref().map_or(Slot::Neck, |e| e.slot);

        // equip with no preferred slot
        let to_add = entity.borrow_mut().actor.equip(item, None);

        for item in to_add {
            stash.borrow_mut().add_item(1, item);
        }

        match slot {
            Slot::HeldMain | Slot::HeldOff => {
                let mgr = GameState::turn_manager();
                let cbs = entity.borrow().callbacks(&mgr.borrow());
                cbs.iter().for_each(|cb| cb.on_held_changed());
            },
            _ => (),
        }
    }))
}

pub fn buy_item_cb(merchant_id: &str, index: usize) -> Callback {
    let merchant_id = merchant_id.to_string();
    Callback::with(Box::new(move || {
        let area_state = GameState::area_state();
        let mut area_state = area_state.borrow_mut();

        let mut merchant = area_state.get_merchant_mut(&merchant_id);
        let merchant = match merchant {
            None => return,
            Some(ref mut merchant) => merchant,
        };

        let value = match merchant.items().get(index) {
            None => return,
            Some(&(_, ref item_state)) => merchant.get_buy_price(item_state),
        };

        if GameState::party_coins() < value {
            return;
        }

        if let Some(item_state) = merchant.remove(index) {
            GameState::add_party_coins(-value);
            let stash = GameState::party_stash();
            stash.borrow_mut().add_item(1, item_state);
        }
    }))
}

pub fn sell_item_cb(entity: &Rc<RefCell<EntityState>>, index: usize) -> Callback {
    let entity = Rc::clone(entity);
    Callback::new(Rc::new(move |widget, _| {
        let (root, root_view) = Widget::parent_mut::<RootView>(widget);
        let merchant = match root_view.get_merchant_window(&root) {
            None => return,
            Some(ref window) => {
                let merchant_window = Widget::kind_mut::<MerchantWindow>(&window);
                merchant_window.merchant_id().to_string()
            }
        };

        let area_state = GameState::area_state();
        let mut area_state = area_state.borrow_mut();
        let mut merchant = area_state.get_merchant_mut(&merchant);
        let merchant = match merchant {
            None => return,
            Some(ref mut merchant) => merchant,
        };

        let stash = GameState::party_stash();
        let item_state = stash.borrow_mut().remove_item(index);
        if let Some(item_state) = item_state {
            let value = merchant.get_sell_price(&item_state);
            GameState::add_party_coins(value);
            merchant.add(item_state);
        }

        let actor = &entity.borrow().actor;
        actor.listeners.notify(actor);
    }))
}

pub fn identify_item_cb(entity: &Rc<RefCell<EntityState>>, index: usize) -> Callback {
    let entity = Rc::clone(entity);
    Callback::new(Rc::new(move |widget, _| {
        let (root, root_view) = Widget::parent_mut::<RootView>(widget);
        let merchant = match root_view.get_merchant_window(&root) {
            None => return,
            Some(ref window) => {
                let merchant_window = Widget::kind_mut::<MerchantWindow>(&window);
                merchant_window.merchant_id().to_string()
            }
        };

        let area_state = GameState::area_state();
        let area_state = area_state.borrow();

        if let Some(merchant) = area_state.get_merchant(&merchant) {

            let stash = GameState::party_stash();
            if let Some(item_state) = stash.borrow_mut().identify(index) {
                let price = merchant.get_sell_price(&item_state) as f32;
                let charge = (price * merchant.identify_fraction.ceil()) as i32;
                if GameState::party_coins() < charge {
                    return;
                }
                GameState::add_party_coins(-charge);
            }

            let actor = &entity.borrow().actor;
            actor.listeners.notify(actor);
        }
    }))
}

pub fn drop_item_cb(entity: &Rc<RefCell<EntityState>>, index: usize) -> Callback {
    let entity = Rc::clone(entity);
    Callback::new(Rc::new(move |widget, _| {
        let stash = GameState::party_stash();
        let item = stash.borrow_mut().remove_item(index);
        if let Some(item) = item {
            drop_item(widget, &entity, item);
        }
    }))
}

fn drop_item(widget: &Rc<RefCell<Widget>>, entity: &Rc<RefCell<EntityState>>, item: ItemState) {
    let (root, root_view) = Widget::parent_mut::<RootView>(widget);
    match root_view.get_prop_window(&root) {
        None => drop_to_ground(entity, item),
        Some(ref window) => {
            let prop_window = Widget::kind_mut::<PropWindow>(&window);
            let prop_index = prop_window.prop_index();
            drop_to_prop(item, prop_index);
        }
    }
}

fn drop_to_prop(item: ItemState, prop_index: usize) {
    let area_state = GameState::area_state();
    let mut area_state = area_state.borrow_mut();
    if !area_state.props().index_valid(prop_index) {
        return;
    }

    let prop_state = area_state.props_mut().get_mut(prop_index);

    prop_state.add_item(item);
}

fn drop_to_ground(entity: &Rc<RefCell<EntityState>>, item: ItemState) {
    let p = entity.borrow().location.to_point();
    let area_state = GameState::area_state();
    let mut area_state = area_state.borrow_mut();

    match area_state.props_mut().check_or_create_container(p.x, p.y) {
        None => (),
        Some(index) => {
            let prop = area_state.props_mut().get_mut(index);
            prop.add_item(item);
        }
    }
}

pub fn unequip_and_drop_item_cb(entity: &Rc<RefCell<EntityState>>, slot: Slot) -> Callback {
    let entity = Rc::clone(entity);
    Callback::new(Rc::new(move |widget, _| {
        let item = entity.borrow_mut().actor.unequip(slot);
        if let Some(item) = item {
            drop_item(widget, &entity, item);
        }

        match slot {
            Slot::HeldMain | Slot::HeldOff => {
                let mgr = GameState::turn_manager();
                let cbs = entity.borrow().callbacks(&mgr.borrow());
                cbs.iter().for_each(|cb| cb.on_held_changed());
            },
            _ => (),
        }
    }))
}

pub fn unequip_item_cb(entity: &Rc<RefCell<EntityState>>, slot: Slot) -> Callback {
    let entity = Rc::clone(entity);
    Callback::with(Box::new(move || {
        let item = entity.borrow_mut().actor.unequip(slot);
        if let Some(item) = item {
            let stash = GameState::party_stash();
            stash.borrow_mut().add_item(1, item);
        }

        match slot {
            Slot::HeldMain | Slot::HeldOff => {
                let mgr = GameState::turn_manager();
                let cbs = entity.borrow().callbacks(&mgr.borrow());
                cbs.iter().for_each(|cb| cb.on_held_changed());
            },
            _ => (),
        }
    }))
}
