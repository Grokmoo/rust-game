//  This file is part of Sulis, a turn based RPG written in Rust.
//  Copyright 2019 Jared Stephen
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

use rlua::{self, Context, UserData, UserDataMethods};

use crate::script::*;
use crate::{area_feedback_text::ColorKind, AreaFeedbackText, EntityState, GameState};
use sulis_module::{ability, Item, ItemState, Module};

/// A kind of Item, represented by its owner (Stash, QuickSlot, or a generic
/// item with a specified ID)
#[derive(Clone, Debug)]
pub enum ScriptItemKind {
    Stash(usize),
    Quick(QuickSlot),
    WithID(String),
}

impl ScriptItemKind {
    pub fn item_checked(&self, parent: &Rc<RefCell<EntityState>>) -> Option<ItemState> {
        match self {
            ScriptItemKind::Stash(index) => {
                let stash = GameState::party_stash();
                let stash = stash.borrow();
                match stash.items().get(*index) {
                    None => None,
                    Some(&(_, ref item)) => Some(item.clone()),
                }
            }
            ScriptItemKind::Quick(slot) => match parent.borrow().actor.inventory().quick(*slot) {
                None => None,
                Some(item) => Some(item.clone()),
            },
            ScriptItemKind::WithID(id) => match Module::item(id) {
                None => None,
                Some(item) => Some(ItemState::new(item, None, true)),
            },
        }
    }

    pub fn item(&self, parent: &Rc<RefCell<EntityState>>) -> ItemState {
        match self {
            ScriptItemKind::Stash(index) => {
                let stash = GameState::party_stash();
                let stash = stash.borrow();
                match stash.items().get(*index) {
                    None => unreachable!(),
                    Some(&(_, ref item)) => item.clone(),
                }
            }
            ScriptItemKind::Quick(slot) => match parent.borrow().actor.inventory().quick(*slot) {
                None => unreachable!(),
                Some(item) => item.clone(),
            },
            ScriptItemKind::WithID(id) => match Module::item(id) {
                None => unreachable!(),
                Some(item) => ItemState::new(item, None, true),
            },
        }
    }
}

/// A ScriptItem, representing a specific item in a player or creature inventory,
/// quick slot, or the party stash, depending on the `ScriptItemKind`.
/// This is passed as the `item` field when using usable items with an associated
/// script.
///
/// # `activate(target: ScriptEntity)`
/// Activates this usable item.  This will remove the AP associated with using this
/// item from the specified `target`.  If the item is consumable, the item will be
/// consumed on calling this method.
///
/// This method is generally used when called from the `on_activate` script of a
/// usable item, once the script has determined that the item should definitely be
/// used.
///
/// # `name() -> String`
/// Returns the name of this Item.
///
/// # `duration() -> Int`
/// Returns the duration, in rounds, of this item, as defined in the item's resource
/// definition.  How this value is used (or not) is up to the script to define.
///
/// # `create_callback(parent: ScriptEntity)`
/// Creates a `ScriptCallback` with the specified parent for this item.  Methods
/// can then be added to the ScriptCallback to cause it to be called when certain
/// events happen.  These methods will be called from this item's script, as
/// defined in its resource file.
#[derive(Clone)]
pub struct ScriptItem {
    parent: usize,
    kind: ScriptItemKind,
    id: String,
    name: String,
    ap: u32,
}

impl ScriptItem {
    pub fn new(parent: &Rc<RefCell<EntityState>>, kind: ScriptItemKind) -> Result<ScriptItem> {
        let item = match kind.item_checked(parent) {
            None => {
                return Err(rlua::Error::FromLuaConversionError {
                    from: "ScriptItem",
                    to: "Item",
                    message: Some(format!("Item with kind {:?} does not exist", kind)),
                });
            }
            Some(item) => item,
        };

        let ap = match &item.item.usable {
            None => 0,
            Some(usable) => usable.ap,
        };

        Ok(ScriptItem {
            parent: parent.borrow().index(),
            kind,
            id: item.item.id.to_string(),
            name: item.get_name(),
            ap,
        })
    }

    pub fn kind(&self) -> ScriptItemKind {
        self.kind.clone()
    }

    pub fn try_item(&self) -> Result<Rc<Item>> {
        let parent = ScriptEntity::new(self.parent).try_unwrap()?;
        let item = self.kind.item_checked(&parent);

        match item {
            None => Err(rlua::Error::FromLuaConversionError {
                from: "ScriptItem",
                to: "Item",
                message: Some(format!(
                    "The item '{}' no longer exists in the parent",
                    self.id
                )),
            }),
            Some(item_state) => Ok(Rc::clone(&item_state.item)),
        }
    }
}

impl UserData for ScriptItem {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("activate", &activate_item);
        methods.add_method("name", |_, item, ()| Ok(item.name.to_string()));
        methods.add_method("duration", |_, item, ()| {
            let item = item.try_item()?;
            match &item.usable {
                None => Ok(0),
                Some(usable) => match usable.duration {
                    ability::Duration::Rounds(amount) => Ok(amount),
                    _ => Ok(0),
                },
            }
        });
        methods.add_method("create_callback", |_, item, parent: ScriptEntity| {
            let index = parent.try_unwrap_index()?;
            let cb_data = CallbackData::new_item(index, item.id.to_string());
            Ok(cb_data)
        });
    }
}

fn activate_item(_lua: Context, script_item: &ScriptItem, target: ScriptEntity) -> Result<()> {
    let item = script_item.try_item()?;
    let target = target.try_unwrap()?;

    let mgr = GameState::turn_manager();
    if mgr.borrow().is_combat_active() {
        target.borrow_mut().actor.remove_ap(script_item.ap);
    }

    let area = GameState::area_state();
    let name = item.name.to_string();

    let mut feedback = AreaFeedbackText::with_target(&target.borrow(), &area.borrow());
    feedback.add_entry(name, ColorKind::Info);
    area.borrow_mut().add_feedback_text(feedback);

    match item.usable {
        None => unreachable!(),
        Some(ref usable) => {
            if usable.consumable {
                let parent = ScriptEntity::new(script_item.parent).try_unwrap()?;
                match &script_item.kind {
                    ScriptItemKind::Quick(slot) => {
                        let item = parent.borrow_mut().actor.clear_quick(*slot);
                        add_another_to_quickbar(&parent, item, *slot);
                    }
                    ScriptItemKind::Stash(index) => {
                        // throw away item
                        let stash = GameState::party_stash();
                        let _ = stash.borrow_mut().remove_item(*index);
                    }
                    ScriptItemKind::WithID(_) => (),
                };
            }
        }
    }

    Ok(())
}

fn add_another_to_quickbar(
    parent: &Rc<RefCell<EntityState>>,
    item: Option<ItemState>,
    slot: QuickSlot,
) {
    if !parent.borrow().is_party_member() {
        return;
    }

    let item = match item {
        None => return,
        Some(item) => item,
    };

    let stash = GameState::party_stash();
    let index = match stash.borrow().items().find_index(&item) {
        None => return,
        Some(index) => index,
    };

    let mut stash = stash.borrow_mut();
    if let Some(item) = stash.remove_item(index) {
        // we know the quick slot is empty because it was just cleared
        let _ = parent.borrow_mut().actor.set_quick(item, slot);
    }
}
