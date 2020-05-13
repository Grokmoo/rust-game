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

use std::str::FromStr;

use rlua::{UserData, UserDataMethods};

use crate::script::*;
use crate::GameState;
use sulis_module::{ability::AIData, ItemKind, QuickSlot, Slot};

/// The inventory of a particular creature, including equipped items
/// and quickslots.
///
/// # `set_locked(locked: Bool)`
/// Sets whether this inventory is locked.  The player will be unable
/// to equip or unequip any items in a locked inventory.
///
/// # `is_locked() -> Bool`
/// Returns whether or not this inventory is locked.  See `set_locked`
///
/// # `has_equipped(slot: String) -> Bool`
/// Returns true if the owning entity has an item equipped in the given
/// slot, false otherwise.  Valid slots are `cloak`, `head`, `torso`,
/// `hands`, `held_main`, `held_off`, `legs`, `feet`, `waist`, `neck`,
/// `fingerMain`, `fingerOff`
///
/// # `equipped_stats(slot: String) -> Table`
/// Returns a table describing the stats of the item in the given slot, or
/// errors if there is no item or the slot is invalid.  See `has_equipped`
/// for valid slots.  The table `stats` includes `stats.name`, `stats.value`,
/// `stats.weight`, `stats.kind`, and `stats.armor_kind` for armor or
/// `stats.weapon_kind` for weapons.
///
/// # `equip_item(item: ScriptStashItem)`
/// Equips the given `item` from the stash into the appropriate inventory
/// slot of the parent.
///
/// # `unequip_item(slot: String) -> ScriptStashItem`
/// Unequips the item in the specified inventory `slot` of the parent.
/// Slot must be one of cloak, head, torso, hands, held_main, held_off,
/// legs, feet, waist, neck, finger_main, finger_off.  Returns the
/// ScriptStashItem representing the unequipped item in the stash, or
/// the invalid item if no item was in the slot
///
/// # `has_equipped_weapon() -> Bool`
/// Returns true if the parent entity currently has a weapon equipped,
/// false otherwise.
///
/// # `has_equipped_shield() -> Bool`
/// Returns true if the parent entity currently has a shield equipped,
/// false otherwise.
///
/// # `has_alt_weapons() -> Bool`
/// Returns true if the parent entity has an item in at least one of its
/// alt weapon slots, meaning it can switch to an alt weapon set.
///
/// # `alt_weapon_style() -> String`
/// Returns the weapon style of the alt weapons that are in the quick slots
/// of the parent creature.  Valid values are `Ranged`, `TwoHanded`, `Single`,
/// `Shielded`, and `DualWielding`
///
/// # `weapon_style() -> String`
/// Returns the weapon style of the currently equipped weapons
/// of the parent creature.  Valid values are `Ranged`, `TwoHanded`, `Single`,
/// `Shielded`, and `DualWielding`
///
/// # `usable_items() -> Table`
/// Returns a table of all items currently in Use QuickSlots for the parent
/// entity.  Each item is represented by a `ScriptUsableItem` in the table.
#[derive(Clone)]
pub struct ScriptInventory {
    parent: ScriptEntity,
}

impl ScriptInventory {
    pub fn new(parent: ScriptEntity) -> ScriptInventory {
        ScriptInventory { parent }
    }
}

macro_rules! try_unwrap {
    ($name:expr => $v:ident) => {
        let parent = $name.parent.try_unwrap()?;
        let parent = parent.borrow();
        let $v = &parent.actor.inventory();
    };
}

impl UserData for ScriptInventory {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("set_locked", |_, data, locked: bool| {
            let entity = data.parent.try_unwrap()?;
            entity.borrow_mut().actor.set_inventory_locked(locked);
            Ok(())
        });

        methods.add_method("is_locked", |_, data, ()| {
            let entity = data.parent.try_unwrap()?;
            let locked = entity.borrow().actor.is_inventory_locked();
            Ok(locked)
        });

        methods.add_method("has_equipped", |_, data, slot: String| {
            let entity = data.parent.try_unwrap()?;
            let slot = match Slot::from_str(&slot) {
                Err(e) => {
                    warn!("{}", e);
                    return Ok(false);
                }
                Ok(slot) => slot,
            };

            let has_equipped = entity.borrow().actor.inventory().equipped(slot).is_some();
            Ok(has_equipped)
        });

        methods.add_method("equipped_stats", |lua, data, slot: String| {
            let entity = data.parent.try_unwrap()?;
            let slot = match Slot::from_str(&slot) {
                Err(e) => {
                    warn!("{}", e);
                    return Err(rlua::Error::FromLuaConversionError {
                        from: "String",
                        to: "Slot",
                        message: Some(format!("{}", e)),
                    });
                }
                Ok(slot) => slot,
            };

            let entity = entity.borrow();
            let item_state = match entity.actor.inventory().equipped(slot) {
                None => {
                    warn!("No item equipped in slot '{:?}'", slot);
                    return Err(rlua::Error::FromLuaConversionError {
                        from: "String",
                        to: "Item",
                        message: Some(format!("No item equipped in slot '{:?}'", slot)),
                    });
                }
                Some(item_state) => item_state,
            };
            let stats = lua.create_table()?;
            stats.set("name", item_state.get_name())?;

            let item = &item_state.item;

            stats.set("value", item.value)?;
            stats.set("weight", item.weight)?;

            match item.kind {
                ItemKind::Armor { kind } => {
                    stats.set("kind", "armor")?;
                    stats.set("armor_kind", format!("{:?}", kind).to_lowercase())?;
                }
                ItemKind::Weapon { kind } => {
                    stats.set("kind", "weapon")?;
                    stats.set("weapon_kind", format!("{:?}", kind).to_lowercase())?;
                }
                ItemKind::Other => stats.set("kind", "other")?,
            }

            Ok(stats)
        });

        methods.add_method("equip_item", |_, data, item: ScriptStashItem| {
            let entity = data.parent.try_unwrap()?;
            let index = item.unwrap_index()?;
            let stash = GameState::party_stash();
            let item = match stash.borrow_mut().remove_item(index) {
                None => {
                    warn!(
                        "Unable to remove item at index '{}' from stash for equip",
                        index
                    );
                    return Ok(());
                }
                Some(item) => item,
            };

            let to_add = entity.borrow_mut().actor.equip(item, None);
            for item in to_add {
                stash.borrow_mut().add_item(1, item);
            }
            Ok(())
        });

        methods.add_method_mut("unequip_item", |_, data, slot: String| {
            let slot = match Slot::from_str(&slot) {
                Err(_) => {
                    return Err(rlua::Error::FromLuaConversionError {
                        from: "String",
                        to: "Slot",
                        message: Some(format!("Invalid slot '{}'", slot)),
                    });
                }
                Ok(slot) => slot,
            };

            let parent = data.parent.try_unwrap()?;
            let item = parent.borrow_mut().actor.unequip(slot);
            let mut index = None;
            if let Some(item) = item {
                let stash = GameState::party_stash();
                index = stash.borrow_mut().add_item(1, item);
            }
            Ok(ScriptStashItem { index })
        });

        methods.add_method("has_equipped_weapon", |_, data, ()| {
            try_unwrap!(data => inv);

            Ok(inv.equipped(Slot::HeldMain).is_some())
        });

        methods.add_method("has_equipped_shield", |_, data, ()| {
            try_unwrap!(data => inv);

            Ok(match inv.equipped(Slot::HeldOff) {
                None => false,
                Some(ref item_state) => match item_state.item.equippable {
                    None => false,
                    Some(ref equippable) => equippable.attack.is_none(),
                },
            })
        });

        methods.add_method("has_alt_weapons", |_, data, ()| {
            try_unwrap!(data => inv);

            let result = inv.quick(QuickSlot::AltHeldMain).is_some()
                || inv.quick(QuickSlot::AltHeldOff).is_some();
            Ok(result)
        });

        methods.add_method("alt_weapon_style", |_, data, ()| {
            try_unwrap!(data => inv);
            Ok(format!("{:?}", inv.alt_weapon_style()))
        });

        methods.add_method("weapon_style", |_, data, ()| {
            try_unwrap!(data => inv);
            Ok(format!("{:?}", inv.weapon_style()))
        });

        methods.add_method("usable_items", |_, data, ()| {
            let parent = data.parent.try_unwrap()?;
            let parent = parent.borrow();
            let mut items = Vec::new();
            for slot in QuickSlot::usable_iter() {
                let item = match parent.actor.inventory().quick(*slot) {
                    None => continue,
                    Some(item) => item,
                };

                let usable = match &item.item.usable {
                    None => unreachable!(),
                    Some(usable) => usable,
                };

                items.push(ScriptUsableItem {
                    parent: data.parent.clone(),
                    slot: *slot,
                    ai: usable.ai.clone(),
                });
            }

            Ok(items)
        });
    }
}

/// A representation of an item in the stash
/// # `is_valid() -> Bool`
/// Returns true if this is a valid item in the stash, false otherwise
#[derive(Clone)]
pub struct ScriptStashItem {
    pub index: Option<usize>,
}

impl ScriptStashItem {
    pub fn unwrap_index(&self) -> Result<usize> {
        match self.index {
            None => Err(rlua::Error::FromLuaConversionError {
                from: "ScriptStashItem",
                to: "Item",
                message: Some("ScriptStashItem is invalid / does not exist.".to_string()),
            }),
            Some(index) => Ok(index),
        }
    }
}

impl UserData for ScriptStashItem {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("is_valid", |_, item, ()| Ok(item.index.is_some()));

        methods.add_method("id", |_, item, ()| match item.index {
            None => Ok(None),
            Some(index) => {
                let stash = GameState::party_stash();
                let stash = stash.borrow();
                match stash.items().get(index) {
                    None => Ok(None),
                    Some((_, item)) => Ok(Some(item.item.id.to_string())),
                }
            }
        });
    }
}

/// A representation of an item that is usable and in a particular QuickSlot for a parent
/// entity.
/// # `ai_data -> Table`
/// Returns a table representing the AI Data of this item, as defined in its resource definition.
/// See `ScriptItem::ai_data`
#[derive(Clone)]
pub struct ScriptUsableItem {
    parent: ScriptEntity,
    pub slot: QuickSlot,
    ai: AIData,
}

impl ScriptUsableItem {
    pub fn ai_data(&self) -> &AIData {
        &self.ai
    }
}

impl UserData for ScriptUsableItem {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("ai_data", |lua, item, ()| {
            let ai_data = lua.create_table()?;
            ai_data.set("priority", item.ai.priority())?;
            ai_data.set("kind", item.ai.kind())?;
            ai_data.set("group", item.ai.group())?;
            ai_data.set("range", item.ai.range())?;

            if let Some(on_activate_fn) = &item.ai.on_activate_fn {
                ai_data.set("on_activate_fn", on_activate_fn.to_string())?;
            }

            Ok(ai_data)
        });

        methods.add_method("name", |_, item, ()| {
            let parent = item.parent.try_unwrap()?;
            let slot = item.slot;

            let parent = parent.borrow();
            let item = parent.actor.inventory().quick(slot);

            let item = match item {
                None => {
                    return Err(rlua::Error::FromLuaConversionError {
                        from: "ScriptUsableItem",
                        to: "Item",
                        message: Some("ScriptUsableItem is invalid / does not exist.".to_string()),
                    });
                }
                Some(item) => item,
            };

            Ok(item.get_name())
        });
    }
}
