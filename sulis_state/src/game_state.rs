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

use std::time;
use std::collections::HashMap;
use std::io::{Error, ErrorKind};
use std::rc::Rc;
use std::cell::{Cell, RefCell};

use rlua;

use sulis_rules::HitKind;
use sulis_core::config::CONFIG;
use sulis_core::util::{self, Point, invalid_data_error};
use sulis_core::io::{GraphicsRenderer};
use sulis_core::ui::{Widget};
use sulis_module::{Ability, Actor, Module, ObjectSize, OnTrigger, area::{Trigger, TriggerKind}};

use {AI, AreaState, ChangeListener, ChangeListenerList, EntityState, Location,
    PathFinder, SaveState, ScriptState, UICallback, MOVE_TO_THRESHOLD, TurnManager};
use script::{script_callback::ScriptHitKind, ScriptEntitySet, ScriptCallback};
use animation::{Animation, MoveAnimation};

thread_local! {
    static TURN_MANAGER: RefCell<TurnManager> = RefCell::new(TurnManager::default());
    static STATE: RefCell<Option<GameState>> = RefCell::new(None);
    static AI: RefCell<AI> = RefCell::new(AI::new());
    static CLEAR_ANIMS: Cell<bool> = Cell::new(false);
    static MODAL_LOCKED: Cell<bool> = Cell::new(false);
    static SCRIPT: ScriptState = ScriptState::new();
    static ANIMATIONS: RefCell<Vec<Box<Animation>>> = RefCell::new(Vec::new());
    static ANIMS_TO_ADD: RefCell<Vec<Box<Animation>>> = RefCell::new(Vec::new());
}

pub struct GameState {
    areas: HashMap<String, Rc<RefCell<AreaState>>>,
    area_state: Rc<RefCell<AreaState>>,
    selected: Vec<Rc<RefCell<EntityState>>>,
    party: Vec<Rc<RefCell<EntityState>>>,

    // listener returns the first selected party member
    party_listeners: ChangeListenerList<Option<Rc<RefCell<EntityState>>>>,
    path_finder: PathFinder,
    ui_callbacks: Vec<UICallback>,
}

macro_rules! exec_script {
    ($func:ident: $($x:ident),*) => {
        let start_time = time::Instant::now();

        let result: Result<(), rlua::Error> = SCRIPT.with(|script_state| {
            script_state.$func($($x, )*)
        });

        if let Err(e) = result {
            warn!("Error executing lua script function");
            warn!("{}", e);
        }

        info!("Script execution time: {}", util::format_elapsed_secs(start_time.elapsed()));
    }
}

impl GameState {
    pub fn load(save_state: SaveState) -> Result<(), Error> {
        let game_state: Result<GameState, Error> = {
            let mut new_indices = HashMap::new();
            let mut areas = HashMap::new();
            for (id, area_save) in save_state.areas {
                let add_indices = id == save_state.current_area;
                let area_state = AreaState::load(&id, &mut new_indices, add_indices, area_save)?;

                areas.insert(id, Rc::new(RefCell::new(area_state)));
            }

            let area_state = match areas.get(&save_state.current_area) {
                Some(ref area) => Ok(Rc::clone(area)),
                None => invalid_data_error(&format!("Unable to load current area '{}'",
                                                    save_state.current_area)),
            }?;

            let path_finder = PathFinder::new(&area_state.borrow().area);

            let mut party = Vec::new();
            for index in save_state.party {
                let new_index = match new_indices.get(&index) {
                    None => invalid_data_error(&format!("Invalid party entity index '{}'",
                                                        index)),
                    Some(index) => Ok(index),
                }?;

                let entity = Rc::clone(&area_state.borrow()
                                       .entities[*new_index].as_ref().unwrap());
                party.push(entity);
            }

            let mut selected = Vec::new();
            for index in save_state.selected {
                let new_index = match new_indices.get(&index) {
                    None => invalid_data_error(&format!("Invalid selected entity index '{}'",
                                                        index)),
                    Some(index) => Ok(index),
                }?;

                let entity = Rc::clone(&area_state.borrow()
                                       .entities[*new_index].as_ref().unwrap());
                selected.push(entity);
            }

            // TODO save load turn timer

            Ok(GameState {
                areas,
                area_state,
                path_finder,
                party,
                selected,
                party_listeners: ChangeListenerList::default(),
                ui_callbacks: Vec::new(),
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

        ANIMATIONS.with(|anims| {
            (*anims.borrow_mut()).clear();
        });

        Ok(())
    }

    pub fn init(pc_actor: Rc<Actor>) -> Result<(), Error> {
        let game_state = GameState::new(pc_actor)?;

        STATE.with(|state| {
            *state.borrow_mut() = Some(game_state);
        });

        let pc = GameState::player();
        let area_state = GameState::area_state();
        area_state.borrow_mut().update_view_visibility();
        area_state.borrow_mut().push_scroll_to_callback(Rc::clone(&pc));
        area_state.borrow_mut().on_load_fired = true;
        let area_state = area_state.borrow();
        GameState::add_ui_callbacks_of_kind(&area_state.area.triggers, TriggerKind::OnCampaignStart, &pc, &pc);
        GameState::add_ui_callbacks_of_kind(&area_state.area.triggers, TriggerKind::OnAreaLoad, &pc, &pc);

        Ok(())
    }

    fn new(pc: Rc<Actor>) -> Result<GameState, Error> {
        let game = Module::game();

        let area_state = GameState::setup_area_state(&game.starting_area)?;

        debug!("Setting up PC {}, with {:?}", &pc.name, &game.starting_location);
        let location = Location::from_point(&game.starting_location, &area_state.borrow().area);

        if !location.coords_valid(location.x, location.y) {
            error!("Starting location coordinates must be valid for the starting area.");
            return Err(Error::new(ErrorKind::InvalidData,
                                  "Unable to create starting location."));
        }

        if !area_state.borrow_mut().add_actor(pc, location, true, None) {
            error!("Player character starting location must be within \
                   area bounds and passable.");
            return Err(Error::new(ErrorKind::InvalidData,
                "Unable to add player character to starting area at starting location"));
        }

        let pc_state = Rc::clone(area_state.borrow().get_last_entity().unwrap());
        pc_state.borrow_mut().actor.init_turn();

        let path_finder = PathFinder::new(&area_state.borrow().area);

        let mut areas: HashMap<String, Rc<RefCell<AreaState>>> = HashMap::new();
        areas.insert(game.starting_area.to_string(), Rc::clone(&area_state));

        let mut party = Vec::new();
        party.push(Rc::clone(&pc_state));

        let mut selected = Vec::new();
        selected.push(Rc::clone(&pc_state));

        Ok(GameState {
            areas,
            area_state: area_state,
            path_finder: path_finder,
            selected,
            party,
            party_listeners: ChangeListenerList::default(),
            ui_callbacks: Vec::new(),
        })
    }

    pub fn set_selected_party_member(entity: Rc<RefCell<EntityState>>) {
        GameState::select_party_members(vec![entity]);
    }

    pub fn clear_selected_party_member() {
        GameState::select_party_members(Vec::new());
    }

    pub fn select_party_members(members: Vec<Rc<RefCell<EntityState>>>) {
        for member in members.iter() {
            if !member.borrow().is_party_member() {
                warn!("Attempted to select non-party member {}", member.borrow().actor.actor.id);
            }
        }

        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let state = state.as_mut().unwrap();

            state.selected.clear();
            // add in party member order
            for party_member in state.party.iter() {
                for member in members.iter() {
                    if Rc::ptr_eq(party_member, member) {
                        state.selected.push(Rc::clone(member));
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

    pub fn selected() -> Vec<Rc<RefCell<EntityState>>> {
        STATE.with(|s| s.borrow().as_ref().unwrap().selected.clone())
    }

    pub fn remove_party_member(entity: Rc<RefCell<EntityState>>) {
        info!("Remove party member {}", entity.borrow().actor.actor.id);
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let state = state.as_mut().unwrap();

            entity.borrow_mut().set_party_member(false);
            state.party.retain(|e| !Rc::ptr_eq(e, &entity));

            state.selected.retain(|e| !Rc::ptr_eq(e, &entity));

            let entity = match state.selected.first() {
                None => None,
                Some(ref entity) => Some(Rc::clone(entity)),
            };
            state.party_listeners.notify(&entity);
        })
    }

    pub fn remove_dead_party_members() {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let state = state.as_mut().unwrap();

            let cur_lens = (state.party.len(), state.selected.len());
            state.party.retain(|e| !e.borrow().actor.is_dead());
            state.selected.retain(|e| !e.borrow().actor.is_dead());

            if state.party.len() != cur_lens.0 || state.selected.len() != cur_lens.1 {
                info!("Removed a dead party member; notifying listeners");
                let entity = match state.selected.first() {
                    None => None,
                    Some(ref entity) => Some(Rc::clone(entity)),
                };
                state.party_listeners.notify(&entity);
            }
        });
    }

    pub fn add_party_member(entity: Rc<RefCell<EntityState>>) {
        info!("Add party member {}", entity.borrow().actor.actor.id);
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let state = state.as_mut().unwrap();

            let turn_timer = state.area_state.borrow().turn_timer();
            if !turn_timer.borrow().is_active() {
                entity.borrow_mut().actor.init_turn();
            }

            entity.borrow_mut().set_party_member(true);
            state.area_state.borrow_mut().compute_pc_visibility(&entity, 0, 0);
            state.party.push(Rc::clone(&entity));

            let entity = match state.selected.first() {
                None => None,
                Some(ref entity) => Some(Rc::clone(entity)),
            };
            state.party_listeners.notify(&entity);
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

    pub fn execute_console_script(script: String) -> String {
        let party = GameState::party();
        let result: Result<String, rlua::Error> = SCRIPT.with(|script_state| {
            script_state.console(script, &party)
        });

        match result {
            Ok(result) => result,
            Err(rlua::Error::FromLuaConversionError { .. }) => "Success".to_string(),
            Err(e) => format!("{}", e),
        }
    }

    pub fn execute_ability_on_activate(parent: &Rc<RefCell<EntityState>>, ability: &Rc<Ability>) {
        exec_script!(ability_on_activate: parent, ability);
    }

    pub fn execute_ability_on_target_select(parent: &Rc<RefCell<EntityState>>, ability: &Rc<Ability>,
                                            targets: Vec<Option<Rc<RefCell<EntityState>>>>,
                                            selected_point: Point, func: &str,
                                            custom_target: Option<Rc<RefCell<EntityState>>>) {
        exec_script!(ability_on_target_select: parent, ability, targets, selected_point,
                     func, custom_target);
    }

    pub fn execute_ability_with_attack_data(parent: &Rc<RefCell<EntityState>>, ability: &Rc<Ability>,
                                        targets: ScriptEntitySet, kind: HitKind,
                                        damage: u32, func: &str) {
        let hit_kind = ScriptHitKind { kind, damage };
        let t = Some(("hit", hit_kind));
        exec_script!(ability_script: parent, ability, targets, t, func);
    }

    pub fn execute_ability_script(parent: &Rc<RefCell<EntityState>>, ability: &Rc<Ability>,
                                  targets: ScriptEntitySet, func: &str) {
        let t: Option<(&str, usize)> = None;
        exec_script!(ability_script: parent, ability, targets, t, func);
    }

    pub fn execute_trigger_script(script_id: &str, func: &str, parent: &Rc<RefCell<EntityState>>,
                                  target: &Rc<RefCell<EntityState>>) {
        exec_script!(trigger_script: script_id, func, parent, target);
    }

    pub fn transition(area_id: &Option<String>, x: i32, y: i32) {
        let p = Point::new(x, y);
        info!("Area transition to {:?} at {},{}", area_id, x, y);

        if let &Some(ref area_id) = area_id {
            // check if area state has already been loaded
            let area_state = GameState::get_area_state(area_id);
            let area_state = match area_state {
                Some(area_state) => area_state,
                None => match GameState::setup_area_state(area_id) {
                    // area state has not already been loaded, try to load it
                    Ok(area_state) => {
                        STATE.with(|state| {
                            let mut state = state.borrow_mut();
                            let state = state.as_mut().unwrap();
                            state.areas.insert(area_id.to_string(), Rc::clone(&area_state));
                        });

                        area_state
                    }, Err(e) => {
                        error!("Unable to transition to '{}'", &area_id);
                        error!("{}", e);
                        return;
                    }
                }
            };

            if !GameState::check_location(&p, &area_state) {
                return;
            }

            STATE.with(|state| {
                let path_finder = PathFinder::new(&area_state.borrow().area);
                state.borrow_mut().as_mut().unwrap().path_finder = path_finder;
                state.borrow_mut().as_mut().unwrap().area_state = area_state;
            });
        } else {
            if !GameState::check_location(&p, &GameState::area_state()) {
                return;
            }
        }

        GameState::set_clear_anims();

        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let state = state.as_mut().unwrap();

            {
                for entity in state.party.iter() {
                    let area_id = entity.borrow().location.area_id.to_string();
                    state.areas.get(&area_id).unwrap().borrow_mut().remove_entity(&entity);
                }
            }

            let base_location = Location::new(x, y, &state.area_state.borrow().area);
            for entity in state.party.iter() {
                entity.borrow_mut().clear_pc_vis();
                let mut cur_location = base_location.clone();
                GameState::find_transition_location(&mut cur_location, &entity.borrow().size,
                                                    &state.area_state.borrow());
                info!("Transitioning {} to {},{}", entity.borrow().actor.actor.name,
                    cur_location.x, cur_location.y);

                match state.area_state.borrow_mut().add_entity(Rc::clone(entity), cur_location) {
                    Ok(_) => (),
                    Err(e) => {
                        warn!("Unable to add party member");
                        warn!("{}", e);
                    }
                }
            }

            state.area_state.borrow_mut().push_scroll_to_callback(Rc::clone(&state.party[0]));

            let area_state = state.area_state.borrow();
            for entity in area_state.entity_iter() {
                entity.borrow_mut().clear_texture_cache();
            }
        });

        let area_state = GameState::area_state();
        let pc = GameState::player();
        let mut area_state = area_state.borrow_mut();
        area_state.update_view_visibility();
        if !area_state.on_load_fired {
            area_state.on_load_fired = true;
            GameState::add_ui_callbacks_of_kind(&area_state.area.triggers, TriggerKind::OnAreaLoad, &pc, &pc);
        }
    }

    fn find_transition_location(location: &mut Location, size: &Rc<ObjectSize>,
                                area_state: &AreaState) {
        let (base_x, base_y) = (location.x, location.y);
        let mut search_size = 0;
        while search_size < 10 {
            // TODO this does a lot of unneccesary checking
            for y in -search_size..search_size+1 {
                for x in -search_size..search_size+1 {
                    if area_state.is_passable_size(size, base_x + x, base_y + y) {
                        location.x = base_x + x;
                        location.y = base_y + y;
                        return;
                    }
                }
            }

            search_size += 1;
        }

        warn!("Unable to find transition locations for all party members");
    }

    fn check_location(p: &Point, area_state: &Rc<RefCell<AreaState>>) -> bool {
        let location = Location::from_point(p, &area_state.borrow().area);
        if !location.coords_valid(location.x, location.y) {
            error!("Location coordinates {},{} are not valid for area {}",
                   location.x, location.y, location.area_id);
            return false;
        }

        true
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
        let area_state = Rc::new(RefCell::new(AreaState::new(area)));
        area_state.borrow_mut().populate();

        Ok(area_state)
    }

    pub fn add_ui_callback(cb: OnTrigger, parent: &Rc<RefCell<EntityState>>,
                           target: &Rc<RefCell<EntityState>>) {
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

    pub fn add_ui_callbacks_of_kind(callbacks: &Vec<Trigger>, kind: TriggerKind,
                                    parent: &Rc<RefCell<EntityState>>, target: &Rc<RefCell<EntityState>>) {
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
        MODAL_LOCKED.with(|c| { c.get() })
    }

    pub fn set_modal_locked(locked: bool) {
        MODAL_LOCKED.with(|c| { c.set(locked) })
    }

    pub fn check_get_ui_callback() -> Option<UICallback> {
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            let state = state.as_mut().unwrap();
            state.ui_callbacks.pop()
        })
    }

    fn check_clear_anims() -> bool {
        CLEAR_ANIMS.with(|c| {
            c.replace(false)
        })
    }

    pub fn set_clear_anims() {
        CLEAR_ANIMS.with(|c| c.set(true));
    }

    pub fn area_state_ids() -> Vec<String> {
        STATE.with(|s| {
            s.borrow().as_ref().unwrap().areas.keys().map(|k| k.to_string()).collect()
        })
    }

    pub fn get_area_state(id: &str) -> Option<Rc<RefCell<AreaState>>> {
        STATE.with(|s| {
            match s.borrow().as_ref().unwrap().areas.get(id) {
                None => None,
                Some(area_state) => Some(Rc::clone(&area_state)),
            }
        })
    }

    pub fn area_state() -> Rc<RefCell<AreaState>> {
        STATE.with(|s| Rc::clone(&s.borrow().as_ref().unwrap().area_state))
    }

    pub fn update(root: &Rc<RefCell<Widget>>, millis: u32) {
        let mut anims_to_add: Vec<Box<Animation>> = ANIMS_TO_ADD.with(|a| {
            let mut anims = a.borrow_mut();

            let to_add = anims.drain(0..).collect();

            to_add
        });

        ANIMATIONS.with(|a| {
            let mut anims = a.borrow_mut();

            anims.append(&mut anims_to_add);

            let mut i = 0;
            while i < anims.len() {
                let retain = anims[i].update(root);

                if retain {
                    i += 1;
                } else {
                    anims.remove(i);
                }
            }
        });

        let (cbs, active_entity) = STATE.with(|s| {
            let mut state = s.borrow_mut();
            let state = state.as_mut().unwrap();

            let mut area_state = state.area_state.borrow_mut();

            let (cbs, active_entity) = area_state.update(millis);

            match active_entity {
                None => (cbs, None),
                Some(ref entity) => (cbs, Some(Rc::clone(entity))),
            }
        });

        GameState::remove_dead_party_members();

        cbs.iter().for_each(|cb| cb.on_round_elapsed());

        if GameState::check_clear_anims() {
            ANIMATIONS.with(|a| {
                let mut anims = a.borrow_mut();
                for anim in anims.iter_mut() {
                    if !anim.is_blocking() { continue; }
                    anim.mark_for_removal();
                }
            });
        }

        if let Some(entity) = active_entity {
            AI.with(|ai| {
                let mut ai = ai.borrow_mut();
                ai.update(entity);
            });
        }
    }

    pub fn draw_graphics_mode(renderer: &mut GraphicsRenderer, offset_x: f32, offset_y: f32,
                              scale_x: f32, scale_y: f32, millis: u32) {
        ANIMATIONS.with(|a| {
            let anims = a.borrow();

            for anim in anims.iter() {
                anim.draw_graphics_mode(renderer, offset_x, offset_y, scale_x, scale_y, millis);
            }
        })
    }

    pub fn has_blocking_animations(entity: &Rc<RefCell<EntityState>>) -> bool {
        ANIMATIONS.with(|a| {
            let anims = a.borrow();

            for anim in anims.iter() {
                if !anim.is_blocking() { continue; }
                if !Rc::ptr_eq(anim.get_owner(), entity) { continue; }

                return true;
            }
            false
        })
    }

    pub fn remove_blocking_animations(entity: &Rc<RefCell<EntityState>>) {
        ANIMATIONS.with(|a| {
            let mut anims = a.borrow_mut();
            for anim in anims.iter_mut() {
                if !anim.is_blocking() { continue; }
                if !Rc::ptr_eq(entity, anim.get_owner()) { continue; }

                anim.mark_for_removal();
            }
        });
    }

    pub fn add_animation(anim: Box<Animation>) {
        ANIMS_TO_ADD.with(|a| {
            let mut anims = a.borrow_mut();

            anims.push(anim);
        });
    }

    /// Returns true if the game is currently in turn mode, false otherwise
    pub fn is_in_turn_mode() -> bool {
        let area_state = GameState::area_state();
        let turn_timer = area_state.borrow().turn_timer();
        let turn_timer = turn_timer.borrow();
        turn_timer.is_active()
    }

    /// Returns true if the PC has the current turn, false otherwise
    pub fn is_pc_current() -> bool {
        let area_state = GameState::area_state();
        let turn_timer = area_state.borrow().turn_timer();
        if let Some(entity) = turn_timer.borrow().current() {
            return entity.borrow().is_party_member();
        }
        false
    }

    pub fn is_current(entity: &Rc<RefCell<EntityState>>) -> bool {
        let area_state = GameState::area_state();
        let turn_timer = area_state.borrow().turn_timer();
        if let Some(ref current) = turn_timer.borrow().current() {
            return Rc::ptr_eq(current, entity);
        }
        false
    }

    fn get_target(entity: &Rc<RefCell<EntityState>>,
                  target: &Rc<RefCell<EntityState>>) -> (f32, f32, f32) {
        let (target_x, target_y) = {
            let target = target.borrow();
            (target.location.x as f32 + (target.size.width / 2) as f32,
             target.location.y as f32 + (target.size.height / 2) as f32)
        };

        let sizes = (entity.borrow().size.diagonal + target.borrow().size.diagonal) / 2.0;
        let mut range = sizes + entity.borrow().actor.stats.attack_distance();

        let area = GameState::area_state();
        let vis_dist = area.borrow().area.vis_dist as f32;
        if range > vis_dist {
            range = vis_dist;
        }

        trace!("Getting move target at {}, {} within {}", target_x, target_y, range);
        (target_x, target_y, range)
    }

    pub fn can_move_towards(entity: &Rc<RefCell<EntityState>>,
                            target: &Rc<RefCell<EntityState>>) -> bool {
        let (x, y, dist) = GameState::get_target(entity, target);
        GameState::can_move_towards_point(entity, Vec::new(), x, y, dist)
    }

    pub fn move_towards(entity: &Rc<RefCell<EntityState>>,
                        target: &Rc<RefCell<EntityState>>) -> bool {
        let (x, y, dist) = GameState::get_target(entity, target);
        GameState::move_towards_point(entity, Vec::new(), x, y, dist, None)
    }

    pub fn can_move_to(entity: &Rc<RefCell<EntityState>>, x: i32, y: i32) -> bool {
        GameState::can_move_towards_point(entity, Vec::new(), x as f32, y as f32, MOVE_TO_THRESHOLD)
    }

    pub fn move_to(entity: &Rc<RefCell<EntityState>>, x: i32, y: i32) -> bool {
        GameState::move_towards_point(entity, Vec::new(), x as f32, y as f32, MOVE_TO_THRESHOLD, None)
    }

    pub fn move_towards_point(entity: &Rc<RefCell<EntityState>>, entities_to_ignore: Vec<usize>,
                              x: f32, y: f32, dist: f32, cb: Option<Box<ScriptCallback>>) -> bool {
        let anim = STATE.with(|s| {
            let mut state = s.borrow_mut();
            let state = state.as_mut().unwrap();
            debug!("Moving '{}' to {},{}", entity.borrow().actor.actor.name, x, y);

            let start_time = time::Instant::now();
            let path = {
                let area_state = state.area_state.borrow();
                match state.path_finder.find(&area_state, entity.borrow(),
                                             entities_to_ignore, x, y, dist) {
                    None => return None,
                    Some(path) => path,
                }
            };
            debug!("Path finding complete in {} secs",
                  util::format_elapsed_secs(start_time.elapsed()));

            let entity = Rc::clone(entity);
            let mut anim = MoveAnimation::new(entity, path, CONFIG.display.animation_base_time_millis);
            anim.set_callback(cb);
            Some(anim)
        });

        match anim {
            None => false,
            Some(anim) => {
                GameState::remove_blocking_animations(entity);
                GameState::add_animation(Box::new(anim));
                true
            }
        }
    }

    pub fn can_move_towards_point(entity: &Rc<RefCell<EntityState>>, entities_to_ignore: Vec<usize>,
                                  x: f32, y: f32, dist: f32) -> bool {
        if entity.borrow().actor.stats.move_disabled { return false; }

        // if entity cannot move even 1 square
        if entity.borrow().actor.ap() < entity.borrow().actor.get_move_ap_cost(1) {
            return false;
        }

        STATE.with(|s| {
            let mut state = s.borrow_mut();
            let state = state.as_mut().unwrap();
            let area_state = state.area_state.borrow();

            let start_time = time::Instant::now();
            let val = match state.path_finder.find(&area_state, entity.borrow(),
                                                   entities_to_ignore, x, y, dist) {
                None => false,
                Some(_) => true,
            };
            debug!("Path finding complete in {} secs",
                  util::format_elapsed_secs(start_time.elapsed()));

            val
        })
    }
}
