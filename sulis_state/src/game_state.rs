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

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{Error, ErrorKind};
use std::rc::Rc;

use sulis_core::config::Config;
use sulis_core::io::{GraphicsRenderer};
use sulis_core::util::{invalid_data_error, ExtInt, Offset, Point, Scale};
use sulis_module::on_trigger::QuestEntryState;
use sulis_module::{
    area::{Destination, PathFinder, Trigger, TriggerKind},
    Actor, ItemState, Module, OnTrigger, Time, MOVE_TO_THRESHOLD,
};

use crate::animation::{particle_generator::Param, Anim, AnimSaveState, AnimState};
use crate::script::{script_cache, script_callback, Script, ScriptCallback, ScriptEntity};
use crate::{
    path_finder, transition_handler, AreaState, ChangeListener, ChangeListenerList, Effect,
    EntityState, Formation, ItemList, Location, PartyStash, QuestStateSet, SaveState, TurnManager,
    UICallback, WorldMapState, AI,
};

thread_local! {
    static TURN_MANAGER: Rc<RefCell<TurnManager>> = Rc::new(RefCell::new(TurnManager::default()));
    static STATE: RefCell<Option<GameState>> = RefCell::new(None);
    static AI: RefCell<AI> = RefCell::new(AI::new());
    static CLEAR_ANIMS: Cell<bool> = Cell::new(false);
    static MODAL_LOCKED: Cell<bool> = Cell::new(false);
    static ANIMATIONS: RefCell<AnimState> = RefCell::new(AnimState::new());
    static ANIMS_TO_ADD: RefCell<Vec<Anim>> = RefCell::new(Vec::new());
    static COMBAT_INACTIVE_TIME: Cell<u32> = Cell::new(0);
}

pub struct GameState {
    areas: HashMap<String, Rc<RefCell<AreaState>>>,
    area_state: Rc<RefCell<AreaState>>,
    world_map: WorldMapState,
    quests: QuestStateSet,
    selected: Vec<Rc<RefCell<EntityState>>>,
    user_zoom: f32,
    party: Vec<Rc<RefCell<EntityState>>>,
    party_formation: Rc<RefCell<Formation>>,
    party_coins: i32,
    party_stash: Rc<RefCell<PartyStash>>,

    // listener returns the first selected party member
    party_listeners: ChangeListenerList<Option<Rc<RefCell<EntityState>>>>,
    party_death_listeners: ChangeListenerList<Vec<Rc<RefCell<EntityState>>>>,
    path_finder: PathFinder,
    ui_callbacks: Vec<UICallback>,
}

const MAX_COMBAT_INACTIVE_TIME: u32 = 5000;

const MIN_ZOOM: f32 = 0.7;
const MAX_ZOOM: f32 = 2.0;

impl GameState {
    pub fn load(save_state: SaveState) -> Result<(), Error> {
        TURN_MANAGER.with(|mgr| {
            mgr.borrow_mut().load(save_state.total_elapsed_millis);
        });
        ANIMATIONS.with(|anims| anims.borrow_mut().clear());
        STATE.with(|state| *state.borrow_mut() = None);
        CLEAR_ANIMS.with(|c| c.set(false));
        MODAL_LOCKED.with(|c| c.set(false));
        ANIMS_TO_ADD.with(|anims| anims.borrow_mut().clear());
        AI.with(|ai| *ai.borrow_mut() = AI::new());
        script_cache::setup().map_err(|e| Error::new(ErrorKind::InvalidInput, e))?;

        let game_state: Result<GameState, Error> = {
            let mut areas = HashMap::new();
            for (id, area_save) in save_state.areas {
                let area_state = AreaState::load(&id, area_save)?;

                areas.insert(id, Rc::new(RefCell::new(area_state)));
            }

            let area_state = match areas.get(&save_state.current_area) {
                Some(ref area) => Ok(Rc::clone(area)),
                None => invalid_data_error(&format!(
                    "Unable to load current area '{}'",
                    save_state.current_area
                )),
            }?;

            let width = area_state.borrow().area.area.width;
            let height = area_state.borrow().area.area.height;
            let path_finder = PathFinder::new(width, height);

            let mut entities = HashMap::new();
            let mut selected = Vec::new();
            let mut party = Vec::new();

            for entity_save in save_state.manager.entities {
                let index = entity_save.index;
                let entity = Rc::new(RefCell::new(EntityState::load(entity_save, &areas)?));
                entities.insert(index, entity);
            }

            for index in save_state.party {
                match entities.get(&index) {
                    None => return invalid_data_error(&format!("Invalid party index {}", index)),
                    Some(ref entity) => party.push(Rc::clone(entity)),
                }
            }

            for index in save_state.selected {
                match entities.get(&index) {
                    None => {
                        return invalid_data_error(&format!("Invalid selected index {}", index))
                    }
                    Some(ref entity) => selected.push(Rc::clone(entity)),
                }
            }

            for entity in entities.values() {
                let area_state = match areas.get(&entity.borrow().location.area_id) {
                    Some(state) => state,
                    None => unreachable!(),
                };

                let is_dead = entity.borrow().actor.is_dead();
                let location = entity.borrow().location.clone();
                area_state
                    .borrow_mut()
                    .load_entity(entity, location, is_dead)?;
            }

            let mut effects = HashMap::new();

            let mgr = GameState::turn_manager();
            mgr.borrow_mut().cur_ai_group_index = save_state.manager.cur_ai_group_index;
            for (key, value) in save_state.manager.ai_groups {
                let index = match key.parse::<usize>() {
                    Ok(val) => val,
                    Err(e) => {
                        let err = Error::new(ErrorKind::InvalidInput, e);
                        return Err(err);
                    }
                };
                mgr.borrow_mut().ai_groups.insert(index, value);
            }

            for effect_save in save_state.manager.effects {
                let old_index = effect_save.index;
                let new_index = mgr.borrow().get_next_effect_index();

                let mut effect = Effect::load(effect_save, new_index, &entities)?;
                if let Some(index) = effect.entity {
                    let entity = match entities.get(&index) {
                        None => {
                            return invalid_data_error(&format!("Invalid effect entity {}", index));
                        }
                        Some(ref entity) => Rc::clone(entity),
                    };

                    // the index has changed with the load
                    effect.entity = Some(entity.borrow().index());

                    let new_idx =
                        mgr.borrow_mut()
                            .add_effect(effect, &entity, Vec::new(), Vec::new());
                    assert!(new_index == new_idx);
                    effects.insert(old_index, new_index);
                    continue;
                }

                if let Some(surface) = effect.surface.clone() {
                    let area = match areas.get(&surface.area_id) {
                        None => {
                            return invalid_data_error(&format!(
                                "Invalid area ID '{}'",
                                surface.area_id
                            ));
                        }
                        Some(area) => area,
                    };

                    let new_idx = mgr.borrow_mut().add_surface(
                        effect,
                        area,
                        surface.points,
                        Vec::new(),
                        Vec::new(),
                    );
                    assert!(new_index == new_idx);
                    effects.insert(old_index, new_index);
                }
            }

            let mut marked = HashMap::new();
            for anim in save_state.anims {
                match anim.load(&entities, &effects, &mut marked) {
                    None => (),
                    Some(anim) => GameState::add_animation(anim),
                }
            }

            let mgr = GameState::turn_manager();
            for (index, vec) in marked {
                mgr.borrow_mut().add_removal_listener_for_effect(index, vec);
            }

            let formation = save_state.formation;

            let party_coins = save_state.coins;

            let mut stash = ItemList::default();
            for item_save in save_state.stash {
                let item = &item_save.item;
                let identified = item.identified;
                let item = match Module::create_get_item(&item.id, &item.adjectives) {
                    None => invalid_data_error(&format!("No item with ID '{}'", item_save.item.id)),
                    Some(item) => Ok(item),
                }?;

                let item = ItemState::new(item, item_save.item.variant, identified);

                stash.add_quantity(item_save.quantity, item);
            }

            let quests = QuestStateSet::load(save_state.quests);
            let mut world_map = save_state.world_map;
            world_map.load();

            mgr.borrow_mut().finish_load();
            area_state.borrow().update_ambient_audio(&mgr.borrow().current_time());
            area_state.borrow().update_music(false, None);

            Ok(GameState {
                areas,
                area_state,
                path_finder,
                party,
                selected,
                user_zoom: save_state.zoom,
                party_formation: Rc::new(RefCell::new(formation)),
                party_coins,
                party_stash: Rc::new(RefCell::new(PartyStash::new(stash))),
                party_listeners: ChangeListenerList::default(),
                party_death_listeners: ChangeListenerList::default(),
                ui_callbacks: Vec::new(),
                world_map,
                quests,
            })
        };

        let game_state = game_state?;
        STATE.with(|state| {
            *state.borrow_mut() = Some(game_state);
        });

        let pc = GameState::player();
        let area_state = GameState::area_state();
        let mut area_state = area_state.borrow_mut();
        area_state.update_view_visibility();
        area_state.push_scroll_to_callback(pc);

        Ok(())
    }

    pub fn init(
        pc_actor: Rc<Actor>,
        party_actors: Vec<Rc<Actor>>,
        flags: HashMap<String, String>,
    ) -> Result<(), Error> {
        ANIMATIONS.with(|anims| anims.borrow_mut().clear());
        CLEAR_ANIMS.with(|c| c.set(false));
        MODAL_LOCKED.with(|c| c.set(false));
        ANIMS_TO_ADD.with(|anims| anims.borrow_mut().clear());
        AI.with(|ai| *ai.borrow_mut() = AI::new());

        TURN_MANAGER.with(|mgr| {
            let rules = Module::rules();
            let starting_time = Module::campaign().starting_time;
            mgr.borrow_mut().load(rules.compute_millis(starting_time));
        });

        script_cache::setup().map_err(|e| Error::new(ErrorKind::InvalidInput, e))?;
        let game_state = GameState::new(pc_actor, party_actors, flags)?;
        STATE.with(|state| {
            *state.borrow_mut() = Some(game_state);
        });

        let pc = GameState::player();
        let area_state = GameState::area_state();
        area_state.borrow_mut().update_view_visibility();
        area_state
            .borrow_mut()
            .push_scroll_to_callback(Rc::clone(&pc));
        area_state.borrow_mut().on_load_fired = true;
        let area_state = area_state.borrow();
        GameState::add_ui_callbacks_of_kind(
            &area_state.area.area.triggers,
            TriggerKind::OnCampaignStart,
            &pc,
            &pc,
        );
        GameState::add_ui_callbacks_of_kind(
            &area_state.area.area.triggers,
            TriggerKind::OnAreaLoad,
            &pc,
            &pc,
        );

        let mgr = GameState::turn_manager();
        area_state.update_ambient_audio(&mgr.borrow().current_time());
        area_state.update_music(false, None);

        Ok(())
    }

    fn new(
        pc: Rc<Actor>,
        party_actors: Vec<Rc<Actor>>,
        flags: HashMap<String, String>,
    ) -> Result<GameState, Error> {
        let party_coins = pc.inventory.pc_starting_coins();
        let mut party_stash = ItemList::default();
        for (qty, item) in pc.inventory.pc_starting_item_iter() {
            party_stash.add_quantity(qty, item);
        }

        let campaign = Module::campaign();

        let area_state = GameState::setup_area_state(&campaign.starting_area)?;

        debug!(
            "Setting up PC {}, with {:?}",
            &pc.name, &campaign.starting_location
        );
        let location =
            Location::from_point(campaign.starting_location, &area_state.borrow().area.area);

        if !location.coords_valid(location.x, location.y) {
            error!("Starting location coordinates must be valid for the starting area.");
            return Err(Error::new(
                ErrorKind::InvalidData,
                "Unable to create starting location.",
            ));
        }

        let index = match area_state
            .borrow_mut()
            .add_actor(pc, location.clone(), None, true, None)
        {
            Err(_) => {
                error!("Player character starting location must be within bounds and passable.");
                return invalid_data_error("Unable to add player character at starting location");
            }
            Ok(index) => index,
        };

        let mgr = GameState::turn_manager();
        let pc_state = mgr.borrow().entity(index);

        pc_state.borrow_mut().actor.init_turn();

        let mut party = Vec::new();
        party.push(Rc::clone(&pc_state));

        for member in party_actors {
            let mut member_location = location.clone();
            transition_handler::find_transition_location(
                &mut member_location,
                &member.race.size,
                &area_state.borrow(),
            );

            let index =
                match area_state
                    .borrow_mut()
                    .add_actor(member, member_location, None, true, None)
                {
                    Err(_) => {
                        error!("Unable to find start location for party member");
                        return invalid_data_error("Unable to find start locations.");
                    }
                    Ok(index) => index,
                };
            let member = mgr.borrow_mut().entity(index);
            member.borrow_mut().actor.init_turn();
            party.push(member);
        }

        for (flag, value) in &flags {
            pc_state.borrow_mut().set_custom_flag(flag, value);
        }

        let width = area_state.borrow().area.area.width;
        let height = area_state.borrow().area.area.height;

        let path_finder = PathFinder::new(width, height);

        let mut areas: HashMap<String, Rc<RefCell<AreaState>>> = HashMap::new();
        areas.insert(campaign.starting_area.to_string(), Rc::clone(&area_state));

        let mut selected = Vec::new();
        selected.push(Rc::clone(&pc_state));

        Ok(GameState {
            user_zoom: Config::default_zoom(),
            areas,
            area_state,
            path_finder,
            selected,
            party,
            party_formation: Rc::new(RefCell::new(Formation::default())),
            party_coins,
            party_stash: Rc::new(RefCell::new(PartyStash::new(party_stash))),
            party_listeners: ChangeListenerList::default(),
            party_death_listeners: ChangeListenerList::default(),
            ui_callbacks: Vec::new(),
            world_map: WorldMapState::new(),
            quests: QuestStateSet::default(),
        })
    }

    pub fn set_world_map_location_visible(location: &str, visible: bool) {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let state = state.as_mut().unwrap();

            state.world_map.set_visible(location, visible);
        })
    }

    pub fn set_world_map_location_enabled(location: &str, enabled: bool) {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let state = state.as_mut().unwrap();

            state.world_map.set_enabled(location, enabled);
        })
    }

    pub fn world_map() -> WorldMapState {
        STATE.with(|state| {
            let state = state.borrow();
            let state = state.as_ref().unwrap();

            state.world_map.clone()
        })
    }

    pub fn quest_state() -> QuestStateSet {
        STATE.with(|state| {
            let state = state.borrow();
            let state = state.as_ref().unwrap();

            state.quests.clone()
        })
    }

    pub fn add_quest_state_change_listener(listener: ChangeListener<QuestStateSet>) {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let state = state.as_mut().unwrap();

            state.quests.listeners.add(listener);
        })
    }

    pub fn get_quest_state(quest: String) -> QuestEntryState {
        STATE.with(|state| {
            let state = state.borrow();
            let state = state.as_ref().unwrap();
            state.quests.state(&quest)
        })
    }

    pub fn get_quest_entry_state(quest: String, entry: String) -> QuestEntryState {
        STATE.with(|state| {
            let state = state.borrow();
            let state = state.as_ref().unwrap();
            state.quests.entry_state(&quest, &entry)
        })
    }

    pub fn set_quest_state(quest: String, entry_state: QuestEntryState) {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let state = state.as_mut().unwrap();
            state.quests.set_state(&quest, entry_state);
        });
    }

    pub fn set_quest_entry_state(quest: String, entry: String, entry_state: QuestEntryState) {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let state = state.as_mut().unwrap();
            state.quests.set_entry_state(&quest, &entry, entry_state);
        })
    }

    pub fn set_user_zoom(mut zoom: f32) {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let state = state.as_mut().unwrap();

            if zoom > MAX_ZOOM {
                zoom = MAX_ZOOM;
            } else if zoom < MIN_ZOOM {
                zoom = MIN_ZOOM;
            }

            state.user_zoom = zoom;
        });
    }

    pub fn user_zoom() -> f32 {
        STATE.with(|state| state.borrow().as_ref().unwrap().user_zoom)
    }

    pub fn turn_manager() -> Rc<RefCell<TurnManager>> {
        TURN_MANAGER.with(|m| Rc::clone(&m))
    }

    pub fn set_selected_party_member(entity: Rc<RefCell<EntityState>>) {
        GameState::select_party_members(vec![entity]);
    }

    pub fn clear_selected_party_member() {
        GameState::select_party_members(Vec::new());
    }

    pub fn select_party_members(mut members: Vec<Rc<RefCell<EntityState>>>) {
        for member in members.iter() {
            if !member.borrow().is_party_member() {
                warn!(
                    "Attempted to select non-party member {}",
                    member.borrow().actor.actor.id
                );
            }
        }

        members.retain(|e| !e.borrow().actor.is_dead());

        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let state = state.as_mut().unwrap();

            state.selected.clear();
            // add in party member order
            for party_member in state.party.iter() {
                for member in members.iter() {
                    if Rc::ptr_eq(party_member, member) {
                        state.selected.push(Rc::clone(member));
                        // GameState::create_selection_animation(&member);
                    }
                }
            }

            let entity = match state.selected.first() {
                None => None,
                Some(ref entity) => Some(Rc::clone(entity)),
            };
            state.party_listeners.notify(&entity);
        })
    }

    pub fn create_damage_animation(entity: &Rc<RefCell<EntityState>>) {
        let time = 200;
        let time_f32 = time as f32 / 1000.0;
        let duration = ExtInt::Int(time);
        let color = [
            Param::with_jerk(0.0, 1.0 / time_f32, 0.0, -1.0 / time_f32),
            Param::fixed(1.0),
            Param::fixed(1.0),
            Param::fixed(1.0),
        ];
        let color_sec = [
            Param::with_jerk(0.0, 0.7 / time_f32, 0.0, -0.7 / time_f32),
            Param::fixed(0.0),
            Param::fixed(0.0),
            Param::fixed(0.0),
        ];
        let anim = Anim::new_entity_color(entity, duration, color, color_sec);
        GameState::add_animation(anim);
    }

    pub fn selected() -> Vec<Rc<RefCell<EntityState>>> {
        STATE.with(|s| s.borrow().as_ref().unwrap().selected.clone())
    }

    pub fn remove_party_member(entity: Rc<RefCell<EntityState>>) {
        info!("Remove party member {}", entity.borrow().actor.actor.id);
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let state = state.as_mut().unwrap();

            entity.borrow_mut().remove_from_party();
            state.party.retain(|e| !Rc::ptr_eq(e, &entity));

            state.selected.retain(|e| !Rc::ptr_eq(e, &entity));

            let entity = match state.selected.first() {
                None => None,
                Some(ref entity) => Some(Rc::clone(entity)),
            };
            state.party_listeners.notify(&entity);
        });

        let area_state = GameState::area_state();
        area_state.borrow_mut().update_view_visibility();
        area_state.borrow_mut().pc_vis_full_redraw();
    }

    fn add_disabled_party_members() {
        for member in GameState::party() {
            {
                let mut member = member.borrow_mut();
                if member.actor.is_dead() && member.actor.is_disabled() {
                    member.actor.set_disabled(false);
                    member.actor.add_hp(1);
                } else {
                    continue;
                }
            }

            let location = member.borrow().location.clone();
            let area_state = GameState::get_area_state(&location.area_id).unwrap();
            let index = member.borrow().index();
            let mut area_state = area_state.borrow_mut();
            area_state.props_mut().remove_matching(
                location.x,
                location.y,
                &member.borrow().actor.actor.name,
            );
            if let Err(e) = area_state.transition_entity_to(&member, index, location) {
                warn!("Error re-adding disabled party member:");
                warn!("{}", e);
            }
            let mgr = GameState::turn_manager();
            mgr.borrow_mut().readd_entity(&member);

            let anim = Anim::new_entity_recover(&member);
            GameState::add_animation(anim);
        }
    }

    fn remove_disabled_party_members() -> bool {
        let mut notify = false;
        for member in GameState::party().iter() {
            {
                let member = member.borrow();
                if !member.actor.is_dead() {
                    continue;
                }
                if member.actor.is_disabled() {
                    continue;
                }
            }

            let script = &Module::campaign().on_party_death_script;
            Script::trigger(&script.id, &script.func, ScriptEntity::from(member));

            {
                let member = member.borrow();
                let prop = match &member.actor.actor.race.pc_death_prop {
                    None => continue,
                    Some(ref prop) => Rc::clone(prop),
                };

                let area_state = GameState::get_area_state(&member.location.area_id).unwrap();
                let x = member.location.x;
                let y = member.location.y;
                let name = member.actor.actor.name.to_string();
                let enabled = member.actor.is_disabled();
                area_state
                    .borrow_mut()
                    .props_mut()
                    .add_at(&prop, x, y, enabled, Some(name));
            }
            notify = true;
        }

        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let state = state.as_mut().unwrap();

            let pc = Rc::clone(&state.party[0]); // don't ever remove the PC
            state.party.retain(|e| {
                if Rc::ptr_eq(&e, &pc) {
                    return true;
                }
                let actor = &e.borrow().actor;
                !actor.is_dead() || actor.is_disabled()
            });
            state.selected.retain(|e| !e.borrow().actor.is_dead());

            if notify {
                info!("Removed or Disabled a dead party member; notifying listeners");
                state.party_death_listeners.notify(&state.party);

                let entity = match state.selected.first() {
                    None => None,
                    Some(ref entity) => Some(Rc::clone(entity)),
                };
                state.party_listeners.notify(&entity);
                true
            } else {
                false
            }
        })
    }

    pub fn handle_disabled_party_members() {
        let update = GameState::remove_disabled_party_members();

        if !GameState::is_combat_active() {
            GameState::add_disabled_party_members();
        }

        if update {
            let area_state = GameState::area_state();
            area_state.borrow_mut().update_view_visibility();
            area_state.borrow_mut().pc_vis_full_redraw();
        }
    }

    pub fn has_party_member(id: &str) -> bool {
        for entity in GameState::party() {
            if entity.borrow().actor.actor.id == id {
                return true;
            }
        }

        false
    }

    pub fn add_party_member(entity: Rc<RefCell<EntityState>>, show_portrait: bool) {
        info!("Add party member {}", entity.borrow().actor.actor.id);
        let mgr = GameState::turn_manager();
        if !mgr.borrow().is_combat_active() {
            entity.borrow_mut().actor.init_turn();
        }

        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let state = state.as_mut().unwrap();

            entity.borrow_mut().add_to_party(show_portrait);
            state
                .area_state
                .borrow_mut()
                .compute_pc_visibility(&entity, 0, 0);
            state.party.push(Rc::clone(&entity));

            let entity = match state.selected.first() {
                None => None,
                Some(ref entity) => Some(Rc::clone(entity)),
            };
            state.party_listeners.notify(&entity);
        });

        let area_state = GameState::area_state();
        area_state.borrow_mut().update_view_visibility();
    }

    pub fn add_party_death_listener(listener: ChangeListener<Vec<Rc<RefCell<EntityState>>>>) {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let state = state.as_mut().unwrap();
            state.party_death_listeners.add(listener);
        })
    }

    pub fn add_party_listener(listener: ChangeListener<Option<Rc<RefCell<EntityState>>>>) {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let state = state.as_mut().unwrap();

            state.party_listeners.add(listener);
        })
    }

    pub fn player() -> Rc<RefCell<EntityState>> {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let state = state.as_mut().unwrap();

            Rc::clone(&state.party[0])
        })
    }

    pub fn party() -> Vec<Rc<RefCell<EntityState>>> {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let state = state.as_mut().unwrap();

            state.party.clone()
        })
    }

    pub fn transition_to(area_id: Option<&str>, p: Option<Point>, offset: Point, time: Time) {
        transition_handler::transition_to(area_id, p, offset, time);
    }

    pub(crate) fn preload_area(area_id: &str) -> Result<(), Error> {
        if GameState::get_area_state(area_id).is_some() {
            return Ok(());
        }

        let area_state = GameState::setup_area_state(area_id)?;

        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let state = state.as_mut().unwrap();
            state.areas.insert(area_id.to_string(), area_state);
        });

        Ok(())
    }

    #[must_use]
    pub(crate) fn set_current_area(area: &Rc<RefCell<AreaState>>) -> bool {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let state = state.as_mut().unwrap();

            if Rc::ptr_eq(&state.area_state, area) {
                return false;
            }

            let width = area.borrow().area.area.width;
            let height = area.borrow().area.area.height;
            let path_finder = PathFinder::new(width, height);
            state.path_finder = path_finder;
            state.area_state = Rc::clone(area);
            true
        })
    }

    fn setup_area_state(area_id: &str) -> Result<Rc<RefCell<AreaState>>, Error> {
        debug!("Setting up area state from {}", &area_id);

        let area = Module::area(&area_id);
        let area = match area {
            Some(a) => a,
            None => {
                error!("Area '{}' not found", &area_id);
                return Err(Error::new(ErrorKind::NotFound, "Unable to create area."));
            }
        };

        let state = AreaState::new(area, None)?;
        let area_state = Rc::new(RefCell::new(state));
        area_state.borrow_mut().populate();

        Ok(area_state)
    }

    pub fn add_ui_callback(
        cb: Vec<OnTrigger>,
        parent: &Rc<RefCell<EntityState>>,
        target: &Rc<RefCell<EntityState>>,
    ) {
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            let state = state.as_mut().unwrap();

            let ui_cb = UICallback {
                on_trigger: cb,
                parent: Rc::clone(parent),
                target: Rc::clone(target),
            };
            state.ui_callbacks.push(ui_cb);
        })
    }

    pub fn add_ui_callbacks_of_kind(
        callbacks: &[Trigger],
        kind: TriggerKind,
        parent: &Rc<RefCell<EntityState>>,
        target: &Rc<RefCell<EntityState>>,
    ) {
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            let state = state.as_mut().unwrap();

            for cb in callbacks.iter() {
                if cb.kind == kind {
                    let ui_cb = UICallback {
                        on_trigger: cb.on_activate.clone(),
                        parent: Rc::clone(parent),
                        target: Rc::clone(target),
                    };
                    state.ui_callbacks.push(ui_cb);
                }
            }
        })
    }

    pub fn is_modal_locked() -> bool {
        MODAL_LOCKED.with(|c| c.get())
    }

    pub fn set_modal_locked(locked: bool) {
        MODAL_LOCKED.with(|c| c.set(locked))
    }

    fn check_clear_anims() -> bool {
        CLEAR_ANIMS.with(|c| c.replace(false))
    }

    pub fn set_clear_anims() {
        CLEAR_ANIMS.with(|c| c.set(true));
    }

    pub fn area_state_ids() -> Vec<String> {
        STATE.with(|s| {
            s.borrow()
                .as_ref()
                .unwrap()
                .areas
                .keys()
                .map(|k| k.to_string())
                .collect()
        })
    }

    pub fn get_area_state(id: &str) -> Option<Rc<RefCell<AreaState>>> {
        STATE.with(|s| match s.borrow().as_ref().unwrap().areas.get(id) {
            None => None,
            Some(area_state) => Some(Rc::clone(&area_state)),
        })
    }

    pub fn area_state() -> Rc<RefCell<AreaState>> {
        STATE.with(|s| Rc::clone(&s.borrow().as_ref().unwrap().area_state))
    }

    #[must_use]
    pub fn update(millis: u32) -> Option<UICallback> {
        let ui_cb = STATE.with(|s| {
            let mut state = s.borrow_mut();
            let state = state.as_mut().unwrap();
            state.ui_callbacks.pop()
        });

        let to_add: Vec<Anim> = ANIMS_TO_ADD.with(|a| {
            let mut anims = a.borrow_mut();
            anims.drain(0..).collect()
        });

        let (update_cbs, complete_cbs) = ANIMATIONS.with(|a| a.borrow_mut().update(to_add, millis));
        update_cbs.into_iter().for_each(|cb| cb.on_anim_update());
        complete_cbs
            .into_iter()
            .for_each(|cb| cb.on_anim_complete());

        let mgr = GameState::turn_manager();
        let update_cbs = mgr.borrow_mut().update(millis);
        script_callback::fire_cbs(update_cbs);

        let triggered_cbs = mgr.borrow_mut().drain_triggered_cbs();
        script_callback::fire_cbs(triggered_cbs);

        let cbs = mgr.borrow_mut().update_entity_move_callbacks();
        script_callback::fire_on_moved(cbs);

        {
            let area_state = GameState::area_state();
            let mut area_state = area_state.borrow_mut();
            area_state.update();
        }

        if GameState::check_clear_anims() {
            ANIMATIONS.with(|a| a.borrow_mut().clear_all_blocking_anims());
        }

        let current = mgr.borrow().current();
        if let Some(entity) = current.as_ref() {
            AI.with(|ai| {
                let mut ai = ai.borrow_mut();
                ai.update(Rc::clone(entity));
            });
        }

        // check if combat has been inactive (stuck) for a while.  if so, end the
        // turn and continue.
        // this should catch any possible turn order hang and move things forward.
        let current_inactive = match current.as_ref() {
            None => true,
            Some(entity) => {
                let actor = &entity.borrow().actor;
                actor.is_dead() || actor.ap() == 0
            }
        };

        let inactive_time = if current_inactive && mgr.borrow().is_combat_active() {
            let prev_time = COMBAT_INACTIVE_TIME.with(|c| c.get());
            let elapsed_time = prev_time + millis;

            if elapsed_time > MAX_COMBAT_INACTIVE_TIME {
                let cbs = mgr.borrow_mut().next();
                script_callback::fire_round_elapsed(cbs);
                0
            } else {
                elapsed_time
            }
        } else {
            0
        };
        COMBAT_INACTIVE_TIME.with(|c| c.set(inactive_time));

        GameState::handle_disabled_party_members();

        let campaign = Module::campaign();
        if let Some(script_data) = &campaign.on_tick_script {
            script_cache::set_report_enabled(false);
            let arg: Option<bool> = None;
            Script::trigger(&script_data.id, &script_data.func, arg);
            script_cache::set_report_enabled(true);
        }

        ui_cb
    }

    pub fn draw_above_entities(
        renderer: &mut dyn GraphicsRenderer,
        offset: Offset,
        scale: Scale,
        millis: u32,
    ) {
        ANIMATIONS.with(|a| {
            a.borrow()
                .draw_above_entities(renderer, offset, scale, millis)
        });
    }

    pub fn draw_below_entities(
        renderer: &mut dyn GraphicsRenderer,
        offset: Offset,
        scale: Scale,
        millis: u32,
    ) {
        ANIMATIONS.with(|a| {
            a.borrow()
                .draw_below_entities(renderer, offset, scale, millis)
        });
    }

    pub fn has_any_blocking_animations() -> bool {
        ANIMATIONS.with(|a| a.borrow().has_any_blocking_anims())
    }

    pub fn has_blocking_animations(entity: &Rc<RefCell<EntityState>>) -> bool {
        ANIMATIONS.with(|a| a.borrow().has_blocking_anims(entity))
    }

    pub fn remove_blocking_animations(entity: &Rc<RefCell<EntityState>>) {
        ANIMATIONS.with(|a| a.borrow_mut().clear_blocking_anims(entity));
    }

    pub fn remove_all_blocking_animations() {
        ANIMATIONS.with(|a| a.borrow_mut().clear_all_blocking_anims());
    }

    pub fn add_animation(anim: Anim) {
        ANIMS_TO_ADD.with(|a| {
            let mut anims = a.borrow_mut();

            anims.push(anim);
        });
    }

    pub fn save_anims() -> Vec<AnimSaveState> {
        ANIMATIONS.with(|a| a.borrow().save_anims())
    }

    /// Returns true if the game is currently in turn mode, false otherwise
    pub fn is_combat_active() -> bool {
        let mgr = GameState::turn_manager();
        let mgr = mgr.borrow();
        mgr.is_combat_active()
    }

    /// Returns true if the PC has the current turn, false otherwise
    pub fn is_pc_current() -> bool {
        let mgr = GameState::turn_manager();
        if let Some(entity) = mgr.borrow().current() {
            return entity.borrow().is_party_member();
        }

        false
    }

    pub fn is_current(entity: &Rc<RefCell<EntityState>>) -> bool {
        let mgr = GameState::turn_manager();
        if let Some(current) = mgr.borrow().current() {
            return Rc::ptr_eq(&current, entity);
        }
        false
    }

    pub fn get_target_dest(entity: &EntityState, target: &EntityState) -> Destination {
        let dist = entity.actor.stats.attack_distance();
        let x = target.location.x as f32;
        let y = target.location.y as f32;
        let w = target.size.width as f32;
        let h = target.size.height as f32;
        let parent_w = entity.size.width as f32;
        let parent_h = entity.size.height as f32;
        Destination {
            x,
            y,
            w,
            h,
            parent_w,
            parent_h,
            dist,
            max_path_len: None,
        }
    }

    pub fn get_point_dest(entity: &EntityState, x: f32, y: f32) -> Destination {
        let dist = MOVE_TO_THRESHOLD;
        let w = entity.size.width as f32;
        let h = entity.size.height as f32;
        let parent_w = entity.size.width as f32;
        let parent_h = entity.size.height as f32;
        Destination {
            x,
            y,
            w,
            h,
            parent_w,
            parent_h,
            dist,
            max_path_len: None,
        }
    }

    pub fn move_towards_dest(
        entity: &Rc<RefCell<EntityState>>,
        entities_to_ignore: &[usize],
        dest: Destination,
        cb: Option<Box<dyn ScriptCallback>>,
    ) -> bool {
        let anim = STATE.with(|s| {
            let mut state = s.borrow_mut();
            let state = state.as_mut().unwrap();

            let area = state.area_state.borrow();
            path_finder::move_towards_point(
                &mut state.path_finder,
                &area,
                entity,
                entities_to_ignore,
                dest,
                cb,
            )
        });

        match anim {
            None => false,
            Some(anim) => {
                GameState::remove_blocking_animations(entity);
                GameState::add_animation(anim);
                true
            }
        }
    }

    pub fn can_move_towards_dest(
        entity: &EntityState,
        entities_to_ignore: &[usize],
        dest: Destination,
    ) -> Option<Vec<Point>> {
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            let state = state.as_mut().unwrap();

            let area = state.area_state.borrow();
            path_finder::can_move_towards_point(
                &mut state.path_finder,
                &area,
                entity,
                entities_to_ignore,
                dest,
            )
        })
    }

    pub fn can_move_ignore_ap(
        entity: &EntityState,
        area: &AreaState,
        entities_to_ignore: &[usize],
        dest: Destination,
    ) -> Option<Vec<Point>> {
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            let state = state.as_mut().unwrap();
            path_finder::can_move_ignore_ap(
                &mut state.path_finder,
                area,
                entity,
                entities_to_ignore,
                dest,
            )
        })
    }

    pub fn party_stash() -> Rc<RefCell<PartyStash>> {
        STATE.with(|s| Rc::clone(&s.borrow().as_ref().unwrap().party_stash))
    }

    pub fn party_coins() -> i32 {
        STATE.with(|s| s.borrow().as_ref().unwrap().party_coins)
    }

    pub fn add_party_coins(amount: i32) {
        STATE.with(|s| s.borrow_mut().as_mut().unwrap().party_coins += amount);
    }

    pub fn party_formation() -> Rc<RefCell<Formation>> {
        STATE.with(|s| {
            let state = s.borrow();
            let state = state.as_ref().unwrap();

            Rc::clone(&state.party_formation)
        })
    }
}
