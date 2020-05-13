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
use std::collections::HashMap;
use std::io::Error;
use std::rc::Rc;

use crate::save_state::ActorSaveState;
use crate::{
    ability_state::DisabledReason, AbilityState, ChangeListenerList, Effect, EntityState,
    GameState, Inventory, PStats,
};
use sulis_core::image::{Image, LayeredImage};
use sulis_core::io::GraphicsRenderer;
use sulis_core::util::{invalid_data_error, ExtInt, Offset, Scale};
use sulis_module::{Ability, Actor, ActorBuilder, Faction, ImageLayer, Module};
use sulis_module::{BonusList, ItemKind, ItemState, QuickSlot, Slot, StatList};

pub struct ActorState {
    pub actor: Rc<Actor>,
    pub stats: StatList,
    pub listeners: ChangeListenerList<ActorState>,
    inventory: Inventory,
    effects: Vec<(usize, BonusList)>,
    image: LayeredImage,
    pub(crate) ability_states: HashMap<String, AbilityState>,
    texture_cache_invalid: bool,
    anim_image_layers: HashMap<ImageLayer, Rc<dyn Image>>,
    p_stats: PStats,
    started_turn_with_no_ap_for_actions: bool,
}

impl ActorState {
    pub fn load(mut save: ActorSaveState, base: Option<ActorBuilder>) -> Result<ActorState, Error> {
        let actor = match base {
            None => match Module::actor(&save.id) {
                None => invalid_data_error(&format!("No actor with id '{}'", save.id)),
                Some(actor) => Ok(actor),
            }?,
            Some(builder) => Rc::new(Module::load_actor(builder)?),
        };

        let attrs = actor.attributes;

        let image = LayeredImage::new(
            actor
                .image_layers()
                .get_list(actor.sex, actor.hair_color, actor.skin_color),
            actor.hue,
        );

        let mut ability_states = HashMap::new();
        for ability in actor.abilities.iter() {
            if ability.ability.active.is_none() {
                continue;
            }

            let mut ability_state = AbilityState::new(&ability.ability);

            match save.ability_states.remove(&ability.ability.id) {
                None => (),
                Some(ability_save) => {
                    ability_state.remaining_duration = ability_save.remaining_duration;
                }
            }

            ability_states.insert(ability.ability.id.to_string(), ability_state);
        }

        // Add any abilities that aren't on the base actor
        for (ability_id, state) in save.ability_states {
            let ability = match Module::ability(&ability_id) {
                None => {
                    return invalid_data_error(&format!(
                        "No ability with ID '{}' for actor '{}'",
                        ability_id, actor.id
                    ));
                }
                Some(ability) => ability,
            };

            let mut ability_state = AbilityState::new(&ability);
            ability_state.remaining_duration = state.remaining_duration;
            ability_states.insert(ability_id, ability_state);
        }

        let mut inventory = Inventory::empty();
        inventory.load(save.equipped, save.quick)?;

        save.p_stats.load(actor.base_class());

        Ok(ActorState {
            actor,
            inventory,
            stats: StatList::new(attrs),
            listeners: ChangeListenerList::default(),
            image,
            effects: Vec::new(),
            ability_states,
            texture_cache_invalid: false,
            p_stats: save.p_stats,
            anim_image_layers: HashMap::new(),
            started_turn_with_no_ap_for_actions: false,
        })
    }

    pub fn new(actor: Rc<Actor>) -> ActorState {
        trace!("Creating new actor state for {}", actor.id);
        let inventory = Inventory::empty();

        let image = LayeredImage::new(
            actor
                .image_layers()
                .get_list(actor.sex, actor.hair_color, actor.skin_color),
            actor.hue,
        );
        let attrs = actor.attributes;

        let mut ability_states = HashMap::new();
        for ability in actor.abilities.iter() {
            let ability = &ability.ability;
            if ability.active.is_none() {
                continue;
            }

            ability_states.insert(ability.id.to_string(), AbilityState::new(ability));
        }

        let mut actor_state = ActorState {
            actor: Rc::clone(&actor),
            inventory,
            stats: StatList::new(attrs),
            listeners: ChangeListenerList::default(),
            image,
            effects: Vec::new(),
            ability_states,
            texture_cache_invalid: false,
            p_stats: PStats::new(&actor),
            anim_image_layers: HashMap::new(),
            started_turn_with_no_ap_for_actions: false,
        };

        actor_state.compute_stats();

        for (slot, item) in actor.inventory.equipped_iter() {
            if !actor_state.can_equip(&item) {
                warn!(
                    "Unable to equip item '{}' for actor '{}'",
                    item.item.id, actor.id
                );
            } else {
                let _ = actor_state.inventory.equip(item, Some(slot));
                // don't deal with any items which have been unequiped as a result
            }
        }

        for (slot, item) in actor.inventory.quick_iter() {
            if !actor_state.inventory.can_set_quick(&item, slot, &actor) {
                warn!(
                    "Unable to set quick item '{}' for actor '{}'",
                    item.item.id, actor.id
                );
            } else {
                let _ = actor_state.inventory.set_quick(item, slot);
                // don't deal with any item which has been removed as a result
            }
        }

        actor_state
    }

    pub fn started_turn_with_no_ap_for_actions(&self) -> bool {
        self.started_turn_with_no_ap_for_actions
    }

    pub fn add_anim_image_layers(&mut self, images: &HashMap<ImageLayer, Rc<dyn Image>>) {
        let mut change = false;
        for (layer, ref image) in images.iter() {
            if let Some(img) = self.anim_image_layers.get(layer) {
                if Rc::ptr_eq(img, image) {
                    continue;
                }
            }
            change = true;
            self.anim_image_layers.insert(*layer, Rc::clone(image));
        }

        if change {
            self.texture_cache_invalid = true;
            self.compute_stats();
        }
    }

    pub fn remove_anim_image_layers(&mut self, images: &HashMap<ImageLayer, Rc<dyn Image>>) {
        for layer in images.keys() {
            self.anim_image_layers.remove(layer);
        }

        self.texture_cache_invalid = true;
        self.compute_stats();
    }

    pub fn p_stats(&self) -> &PStats {
        &self.p_stats
    }

    pub fn is_inventory_locked(&self) -> bool {
        self.p_stats.is_inventory_locked()
    }

    pub fn set_inventory_locked(&mut self, locked: bool) {
        self.p_stats.set_inventory_locked(locked);
    }

    pub fn is_threatened(&self) -> bool {
        self.p_stats.is_threatened()
    }

    pub fn add_threatening(&mut self, index: usize) {
        self.p_stats.add_threatening(index);
    }

    pub fn remove_threatening(&mut self, index: usize) {
        self.p_stats.remove_threatening(index);
    }

    pub fn add_threatener(&mut self, index: usize) {
        let cur = self.p_stats.is_threatened();
        self.p_stats.add_threatener(index);

        if cur != self.p_stats.is_threatened() {
            trace!("Recompute threat on {}", self.actor.name);
            self.compute_stats();
        }
    }

    pub fn remove_threatener(&mut self, index: usize) {
        let cur = self.p_stats.is_threatened();
        self.p_stats.remove_threatener(index);

        if cur != self.p_stats.is_threatened() {
            trace!("Recompute threat on {}", self.actor.name);
            self.compute_stats();
        }
    }

    pub fn faction(&self) -> Faction {
        self.p_stats.faction
    }

    pub fn set_faction(&mut self, faction: Faction) {
        self.p_stats.faction = faction;
    }

    pub fn clone_p_stats(&self) -> PStats {
        self.p_stats.clone()
    }

    pub fn check_texture_cache_invalid(&mut self) -> bool {
        if self.texture_cache_invalid {
            self.texture_cache_invalid = false;
            true
        } else {
            false
        }
    }

    pub fn current_class_stat(&self, id: &str) -> ExtInt {
        *self
            .p_stats
            .current_class_stats
            .get(id)
            .unwrap_or(&ExtInt::Int(0))
    }

    pub fn current_uses_per_day(&self, ability_group: &str) -> ExtInt {
        *self
            .p_stats
            .current_group_uses_per_day
            .get(ability_group)
            .unwrap_or(&ExtInt::Int(0))
    }

    pub fn current_uses_per_encounter(&self, ability_group: &str) -> ExtInt {
        *self
            .p_stats
            .current_group_uses_per_encounter
            .get(ability_group)
            .unwrap_or(&ExtInt::Int(0))
    }

    pub fn ability_state(&mut self, id: &str) -> Option<&mut AbilityState> {
        self.ability_states.get_mut(id)
    }

    /// Returns true if the parent can swap weapons, false otherwise
    pub fn can_swap_weapons(&self) -> bool {
        if self.p_stats.is_inventory_locked() {
            return false;
        }

        self.p_stats.ap() >= Module::rules().swap_weapons_ap
    }

    /// Returns true if this actor can use the item in the specified quick slot
    /// now - which includes having sufficient AP, false otherwise
    pub fn can_use_quick(&self, slot: QuickSlot) -> bool {
        match self.inventory.quick(slot) {
            None => false,
            Some(ref item) => self.can_use(item),
        }
    }

    /// Returns true if this actor can use the item at some point - not
    /// taking AP into consideration, false otherwise
    pub fn can_use_sometime(&self, item_state: &ItemState) -> bool {
        if item_state.item.usable.is_none() {
            return false;
        }

        if !item_state.item.meets_prereqs(&self.actor) {
            return false;
        }

        true
    }

    /// Returns true if the specified item can be used now - which includes
    /// having sufficient AP, false otherwise
    pub fn can_use(&self, item_state: &ItemState) -> bool {
        if !item_state.item.meets_prereqs(&self.actor) {
            return false;
        }

        match &item_state.item.usable {
            None => false,
            Some(usable) => self.p_stats.ap() >= usable.ap,
        }
    }

    fn group_has_uses(&self, group_id: &str) -> bool {
        if self.current_uses_per_encounter(group_id).greater_than(0) {
            return true;
        }

        self.current_uses_per_day(group_id).greater_than(0)
    }

    /// Returns an enum with the reason the ability cannot be activted, or
    /// DisabledReason::Enabled if it can.  For modes, if it can be activated
    /// or deactivated.
    pub fn can_toggle(&self, id: &str) -> DisabledReason {
        use DisabledReason::*;

        if self.stats.abilities_disabled {
            return AbilitiesDisabled;
        }

        match self.ability_states.get(id) {
            None => NoSuchAbility,
            Some(ref state) => {
                if state.is_active_mode() {
                    return Enabled;
                }

                if self.p_stats.ap() < state.activate_ap() {
                    return NotEnoughAP;
                }

                if !self.group_has_uses(&state.group) {
                    return NoAbilityGroupUses;
                }

                if !self.p_stats.has_required_class_stats(&state.ability) {
                    return NotEnoughClassStat;
                }

                state.is_available(&self.stats, &self.current_active_modes())
            }
        }
    }

    pub fn can_activate(&self, id: &str) -> bool {
        if self.stats.abilities_disabled {
            return false;
        }

        match self.ability_states.get(id) {
            None => false,
            Some(ref state) => {
                if self.p_stats.ap() < state.activate_ap() {
                    return false;
                }

                if !self.group_has_uses(&state.group) {
                    return false;
                }

                if !self.p_stats.has_required_class_stats(&state.ability) {
                    return false;
                }

                state.is_available(&self.stats, &self.current_active_modes())
                    == DisabledReason::Enabled
            }
        }
    }

    pub fn deactivate_ability_state(&mut self, id: &str) {
        match self.ability_states.get_mut(id) {
            None => (),
            Some(ref mut state) => {
                state.deactivate();

                let mgr = GameState::turn_manager();
                let mut mgr = mgr.borrow_mut();

                for (index, _) in self.effects.iter() {
                    let effect = mgr.effect_mut(*index);
                    if effect.deactivates_with(id) {
                        effect.mark_for_removal();
                    }
                }
            }
        }
    }

    pub fn activate_ability_state(&mut self, id: &str) {
        let state = match self.ability_states.get_mut(id) {
            None => return,
            Some(state) => state,
        };
        state.activate();

        let decrement_uses = !self.stats.free_ability_group_use;

        if decrement_uses {
            let per_enc = *self
                .p_stats
                .current_group_uses_per_encounter
                .get(&state.group)
                .unwrap_or(&ExtInt::Int(0));

            // take one use from per encounter if available, otherwise take from per day
            if per_enc.is_zero() {
                let per_day = *self
                    .p_stats
                    .current_group_uses_per_day
                    .get(&state.group)
                    .unwrap_or(&ExtInt::Int(0));
                self.p_stats
                    .current_group_uses_per_day
                    .insert(state.group.to_string(), per_day - 1);
            } else {
                self.p_stats
                    .current_group_uses_per_encounter
                    .insert(state.group.to_string(), per_enc - 1);
            }
        }
    }

    pub fn current_active_modes(&self) -> Vec<&str> {
        let mut result = Vec::new();
        for state in self.ability_states.values() {
            if state.is_active_mode() {
                result.push(&state.ability.id[..]);
            }
        }

        result
    }

    pub fn effects_iter(&self) -> impl Iterator<Item = &usize> {
        self.effects.iter().map(|(index, _)| index)
    }

    pub fn replace_actor(&mut self, new_actor: Actor) {
        self.actor = Rc::new(new_actor);

        for ability in self.actor.abilities.iter() {
            let ability = &ability.ability;
            if ability.active.is_none() {
                continue;
            }
            if self.ability_states.contains_key(&ability.id) {
                continue;
            }

            let mut ability_state = AbilityState::new(ability);
            ability_state.newly_added_ability = true;
            self.ability_states
                .insert(ability.id.to_string(), ability_state);
        }

        let mut to_remove = Vec::new();
        for id in self.ability_states.keys() {
            let mut found = false;
            for owned in self.actor.abilities.iter() {
                if &owned.ability.id == id {
                    found = true;
                    break;
                }
            }

            if !found {
                to_remove.push(id.clone());
            }
        }

        for id in to_remove {
            self.ability_states.remove(&id);
        }

        self.compute_stats();
    }

    pub fn draw(
        &self,
        renderer: &mut dyn GraphicsRenderer,
        offset: Offset,
        scale: Scale,
        millis: u32,
    ) {
        self.image.draw(renderer, offset, scale, millis);
    }

    pub fn draw_to_texture(
        &self,
        renderer: &mut dyn GraphicsRenderer,
        texture_id: &str,
        offset: Offset,
        scale: Scale,
    ) {
        self.image
            .draw_to_texture(renderer, texture_id, offset, scale);
    }

    pub fn has_ap_to_attack(&self) -> bool {
        self.p_stats.ap() >= self.stats.attack_cost as u32
    }

    pub fn has_ap_for_any_action(&self) -> bool {
        self.ap() >= self.get_move_ap_cost(1) || self.has_ap_to_attack()
    }

    /// Sets the specified item as the item at the quick slot.  Returns the
    /// item that was previously there, if it was present
    #[must_use]
    pub fn set_quick(&mut self, item: ItemState, slot: QuickSlot) -> Option<ItemState> {
        let item = self.inventory.set_quick(item, slot);
        self.listeners.notify(&self);
        item
    }

    /// Clears any item at the specified quick slot.  Returns the item
    /// if it is present
    #[must_use]
    pub fn clear_quick(&mut self, slot: QuickSlot) -> Option<ItemState> {
        let item = self.inventory.clear_quick(slot);
        self.listeners.notify(&self);
        item
    }

    /// Should only be called by swap_weapon_set in EntityState
    pub(crate) fn do_swap_weapons(&mut self) -> bool {
        let swap_ap = Module::rules().swap_weapons_ap;
        if self.ap() < swap_ap {
            return false;
        }
        self.inventory.swap_weapon_set();
        self.compute_stats();
        self.texture_cache_invalid = true;

        if GameState::is_combat_active() {
            self.remove_ap(swap_ap);
        }
        true
    }

    /// Attempts to equip the specified item to this actor's inventory.
    /// Returns a list of free items that need to be placed somewhere.
    /// If the equip action was not possible, this will include the item that was
    /// passed in.  Otherwise, it will include any items that were unequipped
    /// in order to equip the new item.  This will frequently be an empty list
    #[must_use]
    pub fn equip(&mut self, item: ItemState, preferred_slot: Option<Slot>) -> Vec<ItemState> {
        if !self.can_equip(&item) {
            return vec![item];
        }

        let unequipped = self.inventory.equip(item, preferred_slot);
        self.compute_stats();
        self.texture_cache_invalid = true;
        unequipped
    }

    pub fn can_equip(&self, item: &ItemState) -> bool {
        if self.p_stats.is_inventory_locked() {
            return false;
        }

        self.inventory.can_equip(&item, &self.stats, &self.actor)
    }

    pub fn can_unequip(&self, _slot: Slot) -> bool {
        if self.p_stats.is_inventory_locked() {
            return false;
        }

        !GameState::is_combat_active()
    }

    #[must_use]
    pub fn unequip(&mut self, slot: Slot) -> Option<ItemState> {
        if self.p_stats.is_inventory_locked() {
            return None;
        }

        let item = self.inventory.unequip(slot);
        self.compute_stats();
        self.texture_cache_invalid = true;
        item
    }

    pub fn inventory(&self) -> &Inventory {
        &self.inventory
    }

    pub fn is_dead(&self) -> bool {
        self.hp() <= 0
    }

    pub fn check_death(parent: &Rc<RefCell<EntityState>>, target: &Rc<RefCell<EntityState>>) {
        if target.borrow().actor.hp() > 0 {
            return;
        }

        let area_state = GameState::area_state();

        let reward = {
            let target = target.borrow();
            match target.actor.actor.reward {
                None => return,
                Some(ref reward) => reward.clone(),
            }
        };

        debug!(
            "Adding XP {} to '{}'",
            reward.xp,
            parent.borrow().actor.actor.id
        );
        if parent.borrow().is_party_member() {
            for member in GameState::party().iter() {
                member.borrow_mut().add_xp(reward.xp);
            }
        } else {
            parent.borrow_mut().add_xp(reward.xp);
        }

        let loot = match reward.loot {
            None => return,
            Some(ref loot) => loot,
        };

        trace!("Checking for loot drop.");
        let items = loot.generate_with_chance(reward.loot_chance);
        if items.is_empty() {
            return;
        }

        trace!("Dropping loot with {} items", items.len());
        let p = target.borrow().location.to_point();
        let mut area_state = area_state.borrow_mut();

        match area_state.props_mut().check_or_create_container(p.x, p.y) {
            None => (),
            Some(index) => {
                let prop = area_state.props_mut().get_mut(index);
                prop.add_items(items);
            }
        }
    }

    pub fn is_disabled(&self) -> bool {
        self.p_stats.is_disabled()
    }

    pub fn set_disabled(&mut self, disabled: bool) {
        self.p_stats.set_disabled(disabled);
    }

    pub fn has_level_up(&self) -> bool {
        self.p_stats.has_level_up()
    }

    pub fn add_xp(&mut self, xp: u32) {
        self.p_stats.add_xp(xp, &self.actor);
        self.listeners.notify(&self);
    }

    pub fn xp(&self) -> u32 {
        self.p_stats.xp()
    }

    pub fn hp(&self) -> i32 {
        self.p_stats.hp()
    }

    pub fn overflow_ap(&self) -> i32 {
        self.p_stats.overflow_ap()
    }

    pub fn ap(&self) -> u32 {
        self.p_stats.ap()
    }

    pub fn get_move_ap_cost(&self, squares: u32) -> u32 {
        let rules = Module::rules();
        ((rules.movement_ap as f32) / self.stats.movement_rate) as u32 * squares
    }

    pub fn set_overflow_ap(&mut self, ap: i32) {
        self.p_stats.set_overflow_ap(ap);
    }

    pub fn change_overflow_ap(&mut self, ap: i32) {
        let cur_overflow = self.p_stats.overflow_ap();
        self.p_stats.set_overflow_ap(cur_overflow + ap);
    }

    pub(crate) fn add_ap(&mut self, ap: u32) {
        self.p_stats.add_ap(ap);
        self.listeners.notify(&self);
    }

    pub(crate) fn remove_class_stats(&mut self, ability: &Ability) {
        self.p_stats.remove_class_stats(ability);
        self.listeners.notify(&self);
    }

    pub(crate) fn remove_ap(&mut self, ap: u32) {
        self.p_stats.remove_ap(ap);
        self.listeners.notify(&self);
    }

    pub(crate) fn remove_hp(&mut self, hp: u32) {
        self.p_stats.remove_hp(hp);
        self.listeners.notify(&self);
    }

    pub(crate) fn add_hp(&mut self, hp: u32) {
        self.p_stats.add_hp(hp, self.stats.max_hp);
        self.listeners.notify(&self);
    }

    pub(crate) fn remove_class_stat(&mut self, stat: &str, amount: u32) {
        self.p_stats.remove_class_stat(stat, amount);
        self.listeners.notify(&self);
    }

    pub(crate) fn add_class_stat(&mut self, stat: &str, amount: u32) {
        let max = self.stats.class_stat_max(stat);
        self.p_stats.add_class_stat(stat, amount, max);
        self.listeners.notify(&self);
    }

    pub fn elapse_time(&mut self, millis_elapsed: u32, all_effects: &[Option<Effect>]) {
        for (_, ability_state) in self.ability_states.iter_mut() {
            ability_state.update(millis_elapsed);
        }

        let start_len = self.effects.len();
        self.effects
            .retain(|(index, _)| all_effects[*index].is_some());

        if start_len != self.effects.len() {
            self.compute_stats();
        }
    }

    pub fn add_effect(&mut self, index: usize, bonuses: BonusList) {
        info!(
            "Adding effect with index {} to '{}'",
            index, self.actor.name
        );
        self.effects.push((index, bonuses));
        self.compute_stats();
    }

    pub(crate) fn remove_effect(&mut self, index: usize) {
        self.effects.retain(|(i, _)| *i != index);
        self.compute_stats();
    }

    pub fn init_day(&mut self) {
        self.p_stats.init_day(&self.stats);
        self.listeners.notify(&self);
    }

    pub fn end_encounter(&mut self) {
        self.p_stats.end_encounter(&self.stats);
        self.listeners.notify(&self);
    }

    pub fn init_turn(&mut self) {
        debug!(
            "Init turn for '{}' with overflow ap of {}",
            self.actor.name,
            self.overflow_ap()
        );
        self.p_stats.init_turn(&self.stats);

        self.started_turn_with_no_ap_for_actions = !self.has_ap_for_any_action();
        debug!("Initial AP: {}", self.ap());

        self.listeners.notify(&self);
    }

    pub fn end_turn(&mut self) {
        self.p_stats.end_turn();
        self.listeners.notify(&self);
    }

    pub fn compute_stats(&mut self) {
        debug!("Compute stats for '{}'", self.actor.name);
        self.stats = StatList::new(self.actor.attributes);

        let mut layers_override = self.inventory().get_image_layers();
        for (layer, image) in self.anim_image_layers.iter() {
            layers_override.insert(*layer, Rc::clone(image));
        }

        let layers = self.actor.image_layers().get_list_with(
            self.actor.sex,
            &self.actor.race,
            self.actor.hair_color,
            self.actor.skin_color,
            layers_override,
        );
        self.image = LayeredImage::new(layers, self.actor.hue);

        self.stats.add(&self.actor.race.base_stats);

        for &(ref class, level) in self.actor.levels.iter() {
            self.stats.add_multiple(&class.bonuses_per_level, level);
            for (ref group_id, amount) in class.group_uses_per_encounter(level).iter() {
                self.stats
                    .add_single_group_uses_per_encounter(group_id, *amount);
            }

            for (ref group_id, amount) in class.group_uses_per_day(level).iter() {
                self.stats.add_single_group_uses_per_day(group_id, *amount);
            }

            for (stat_id, amount) in class.stats_max(level) {
                self.stats.add_single_class_stat_max(stat_id, amount);
            }
        }

        for ability in self.actor.abilities.iter() {
            let level = ability.level;
            ability.ability.add_bonuses_to(level, &mut self.stats);
        }

        let mut attacks_list = Vec::new();
        for item_state in self.inventory.equipped_iter() {
            let equippable = match &item_state.item.equippable {
                None => continue,
                Some(equippable) => {
                    if let Some(attack) = &equippable.attack {
                        let weapon_kind = match item_state.item.kind {
                            ItemKind::Weapon { kind } => kind,
                            _ => {
                                warn!("Weapon attack belonging to item '{}' with no associated WeaponKind",
                                      item_state.item.id);
                                continue;
                            }
                        };

                        attacks_list.push((attack, weapon_kind));
                    }

                    equippable
                }
            };
            if item_state.identified {
                self.stats.add(&equippable.bonuses);
            }
        }

        for (_, ref bonuses) in self.effects.iter() {
            self.stats.add(bonuses);
        }

        let mut equipped_armor = HashMap::new();
        for slot in Slot::iter() {
            if let Some(ref item_state) = self.inventory.equipped(*slot) {
                if let ItemKind::Armor { kind } = item_state.item.kind {
                    equipped_armor.insert(*slot, kind);
                }
            }
        }

        let weapon_style = self.inventory.weapon_style();
        let is_threatened = self.is_threatened();

        self.stats.finalize(
            &self.actor,
            attacks_list,
            equipped_armor,
            weapon_style,
            is_threatened,
        );

        self.p_stats.recompute_level_up(&self.actor);

        self.listeners.notify(&self);
    }
}
