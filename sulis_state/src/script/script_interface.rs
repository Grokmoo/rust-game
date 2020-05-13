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

use rlua::{self, UserData, UserDataMethods};

use crate::script::*;
use crate::{animation::Anim, AreaState, EntityState, GameState, Location};
use sulis_core::{config::Config};
use sulis_module::on_trigger::{self, QuestEntryState};
use sulis_module::{Faction, ItemState, Module, OnTrigger, Time};

/// The ScriptInterface, accessible in all Lua scripts as the global `game`.
/// The following methods are available on this object (documentation WIP):
///
/// # `play_sfx(id: String, volume: Float (Optional))`
/// Plays the sound effect with the specified ID.  Optionally multiple the
/// sound base volume by the specified volume
///
/// # `current_round() -> Int`
/// Returns the current round, or the total number of rounds of playtime that have elapsed.
/// This number increases by 1 for every complete round of combat, or by 1 for every 5 seconds
/// of out of combat play.
///
/// # `add_time(days: Int, hours: Int (Optional), rounds: Int (Optional))`
/// Adds the specified days, hours, and rounds to the current time.
///
/// # `current_time() -> Table`
/// Returns a table containing the current time.
/// Table entries are `day`, `hour`, and `round`.
///
/// # `party() -> Table<ScriptEntity>`
/// Returns a table containing all current party members.
///
/// # `entity_with_id(id: String) -> Table<ScriptEntity>`
/// Returns a `ScriptEntity` object for the entity with the given unique
/// id, if such an entity can be found.  Otherwise, returns the invalid `ScriptEntity`.  The ID is
/// the unique id associated with the individual entity, which is typically the same as the actor
/// ID for actors that are only used once, and is generated for encounter created actors.
/// ## Examples
/// ```lua
///   entity = game:entity_with_id("id1")
///   if not entity:is_valid() return end
///   game:log("Found entity with name " .. entity:name())
///```
///
/// # `entities_with_ids(ids: Table) -> ScriptEntity`
/// Returns a list of `ScriptEntity` objects for each of the specified
/// ids that are found.  The list may be empty.  Also see `entity_with_id(id)`.
/// ## Examples
/// ```lua
///  entities = game:entities_with_ids({"id1", "id2"})
///  for i = 1, #entities do
///    game:log("Found entity with name " .. entities[i]:name())
///  end
/// ```
///
///# `activate_targeter()`
/// Activates the current targeter, if one exists.  You should first
/// validate the targeter exists with `has_targeter()` and then
/// set a valid position with `check_targeter_position(x, y)`.
///
/// ## Examples
/// ```lua
/// if game:has_targeter() then
///   x = target:x()
///   y = target:y()
///   if game:check_targeter_position(x, y) then
///     game:activate_targeter()
///   end
/// end
/// ```
///
/// # `cancel_targeter()`
/// Deactivates the current targeter.  Throws an error if there is no
/// active targeter.  You should verify a targeter is active with `has_targeter()`
/// before calling this method.
///
/// # `check_targeter_position(x: Int, y: Int) -> bool`
/// Sets the selected coordinates for the targeter and then checks if
/// the position is valid to activate.  See also `activate_targeter()`
///
/// # `get_targeter_affected() -> ScriptEntitySet`
/// Returns the set of entities currently affected by the active targeter,
/// if there is one.  If there is no active targeter, throws an error.  You
/// should verify a targeter is active with `has_targeter()` before calling
/// this method.
///
/// # `get_targeter_selectable() -> ScriptEntitySet`
/// Returns the set of entities that can be selected by the active targeter,
/// if there is one and it is no free select.  You should verify there is an
/// active targeter with `has_targeter()` before calling this method.
///
/// # `is_targeter_free_select() -> Bool`
/// Returns true if the active targeter is free select, false otherwise.
/// You should verify there is an active targeter with `has_targeter()`
/// before calling this method.
///
/// # `has_targeter() -> Bool`
/// Returns true if a targeter is currently active, false otherwise.
/// Useful for AI activating of abilities.
///
/// # `cancel_blocking_anims()`
/// Cancels all current blocking animations on all entities.  Blocking animations
/// are those that normally cause the player to wait for their completion before
/// performing another action.  This includes movement, attacks, and most
/// fixed duration particle effects.
///
/// # `check_ai_activation(entity: ScriptEntity)`
/// Checks all entities for ai activation - i.e. if they can see a hostile,
/// they become AI active.  This can trigger the start of combat.  Useful
/// when a script updates the state of the area in such a way that combat
/// might start (such as unhiding an entity or spawning an encounter).  This
/// is not needed when scripts cause movement, as it is called automatically
/// in those cases.  The entity should be the one whose state has changed.
///
/// # `fade_out_in()`
/// Causes the main view to fade out, then back in again.  This duration of the
/// fades is defined in the theme for the `WindowFade` widget.
///
/// # `init_party_day()`
/// Starts a new day for the player character and party.  This resets all skill
/// uses and sets maximum hit points.  This is normally used in a script when the
/// party rests.
///
/// # `create_menu_selection(value: String)`
/// Creates a ScriptMenuSelection object with the specified value.  Useful to
/// manually generate a callback value without the user actually clicking on it
///
/// # `create_menu(title: String, callback: CallbackData)`
/// Creates a new `ScriptMenu` which can then be built up and finally shown with `show()`.
/// Calls the callback function `on_menu_select` when the user select an option.
///
/// # `show_confirm(message: String, accept: String, cancel: String,
/// id: String, func: String)`
/// Shows a simple confirmation dialog with the specified `message`, and specified text
/// on the `accept` and `cancel` buttons.  If the user cancels, no action is taken.  If the
/// user accepts, the specified `func` is called from the script with `id`.
///
/// # `warn(message: String)`
/// Logs the specified string to the game's output at warn level.
///
/// # `log(message: String)`
/// Logs the specified string to the game's output at info level.  This is primarily useful
/// for debugging purposes.
///
/// # `debug(message: String)`
/// Logs the specified string to game output at debug level.
///
/// # `trace(message: String)`
/// Logs the specified string to game output at trace level.
///
/// # `ap_display_factor() -> Int`
/// Gets the ap display factor, which is the factor that the internal AP representation is
/// divided by when displayed.  Any AP values that are displayed to the user must be
/// divided by this factor.
///
/// # `anim_base_time() -> Float`
/// Returns the animation base time, which affects how long animations last.  Generally,
/// animations should multiply some base time by this factor when determining the duration
/// of animations.  This value is user configurable in the options menu.
///
/// # `atan2(x: Float, y: Float) -> Float`
/// Computes the four quadrant arctan function.  See `f32::atan2`
///
/// # `block_ui(time: Float)`
/// Locks the UI so the player cannot take any additional in game actions (such as movement
/// or combat) for the specified `time` number of seconds.
///
/// # `run_script_delayed(script_id: String, func: String, delay: Float)`
/// Causes the specified `func` from the script with `script_id` to be run after `delay`
/// seconds.  The script is actually run on the first frame after `delay` seconds have
/// elapsed.  The game can normally achieve a comfortable 60 fps on the vast majority of
/// hardware, but be aware that this is not always the case.
///
/// # `create_callback(parent: ScriptEntity, script: String) -> ScriptCallback`
/// Creates a new script callback.  This callback will utilize the specified script
/// file for all methods.  See `ScriptCallback` for more.
///
/// # `set_quest_state(quest: String, state: String)`
/// Sets the specified `quest` to the `state`.  `state` must be one of `Hidden`, `Visible`,
/// `Active`, or `Complete`.  `quest` must be the ID of a valid quest definition.
///
/// # `set_quest_entry_state(quest: String, entry: String, state: String)`
/// Sets the specified `entry` within the specified `quest` to `state`.  `state` must be one
/// of `Hidden`, `Visible, `Active`, or `Complete`.  `quest` must be the ID of a valid quest
/// definition, and `entry` must be an entry within that quest.
///
/// # `get_quest_state(quest: String) -> String`
/// Returns the current `state` of the specified `quest`.  `state` will be one of
/// `Hidden`, `Visible`, `Active`, or `Complete`.
///
/// # `get_quest_entry_state(quest: String, entry: String)`
/// Returns the current `state` of the specified `entry` in the given `quest`.
///
/// # `set_world_map_location_visible(location: String, visible: Bool)`
/// Sets the specified `location` in the world map to the specified `visible`.  The
/// location must be defined in the world_map section of the campaign definition file.
///
/// # `set_world_map_location_enabled(location: String, enabled: Bool)`
/// Sets the specified `location` in the world map `enabled`.  If disabled, a user
/// viewing the world map cannot travel to that location.  The location  must be defined
/// in the world_map section of the campaign definition file.
///
/// # `is_passable(entity: ScriptEntity, x: Int, y: Int) -> Bool`
/// Returns true if the specified coordinates in the current area are passable for
/// the entity, false otherwise.
///
/// # `spawn_actor_at(id: String, x: Int, y: Int, faction: String (Optional)) -> ScriptEntity`
/// Attempts the spawn an instance of the actor with the specified `id` at the
/// coordinates `x`, `y` in the current area.  If successful, returns the
/// ScriptEntity that was just spawned.  If not, returns the invalid ScriptEntity.
/// Optionally, you may set the faction of the spawned actor to the specified value.
/// Must be "Hostile", "Neutral", or "Friendly".  This method can fail if the
/// ID or coordinates are invalid, or if the location is not passable for the entity
///
/// # `spawn_encounter_at(x: Int, y: Int, area_id: String (Optional))`
/// Causes the encounter in the current area at `x`, `y` to spawn entities based
/// on its encounter definition.  If the entities are hostile and within player
/// visibility, will initiate combat.
///
/// # `enable_trigger_at(x: Int, y: Int, area_id: String (Optional))`
/// Sets the trigger in the current area at `x`, `y` to enabled.  This means the
/// trigger will fire when its condition (such as player entering its coordinates)
/// are met.  This method will only have an effect on triggers which are set to
/// be initially_disabled in their defintion, or which have been disabled via
/// `disable_trigger_at`.
///
/// # `disable_trigger_at(x: Int, y: Int, area_id: String(Optional))`
/// Sets the trigger in the current area at `x`, `y` to disabled.  This means the
/// trigger will not fire regardless of whether its condition is met.
///
/// # `enable_prop_at(x: Int, y: Int, area_id: String (Optional))`
/// Sets the prop in the current area at `x`, `y` to enabled.  When enabled, props
/// can be interacted with if relevant for the given prop (doors or containers).
///
/// # `disable_prop_at(x: Int, y: Int, area_id: String (Optional))`
/// Sets the prop in the current area at `x`, `y` to disabled.  When disabled, props
/// cannot be interacted with regardless of whether they otherwise are interactive.
///
/// # `toggle_prop_at(x: Int, y: Int, area_id: String (Optional))`
/// Toggles the enabled / disabled state of the prop at `x`, `y`.  See `enable_prop_at` and
/// `disable_prop_at`
///
/// # `say_line(line: String, target: ScriptEntity (Optional))`
/// The specified `target`, or the player if no target is specified, will say the line
/// of text specified by `line`.  This is represented by the text appearing on the main
/// area view overhead of the target entity.  The text fades away after several seconds.
///
/// # `start_conversation(id: String, target: ScriptEntity (Optional))`
/// Starts the conversation with the specified `id`, with the `target` or the player if the
/// target is not specified.  The conversation is defined in the conversation data file
/// for the relevant id.
///
/// # `show_game_over_window(text: String)`
/// Shows the game over window, indicating that the player cannot continue
/// in the current module without loading.  This can be used to show victory
/// or defeat.  The specified `text` is displayed.
///
/// # `create_module_export(id: String) -> ModuleExport`
/// Creates a `ModuleExport` object which can be used to configure transfering data
/// and loading a new campaign/module.  Once it is set up, the new campaign is loaded
/// by calling `activate` on the returned object.
///
/// # `player -> ScriptEntity`
/// Returns a reference to the player character ScriptEntity.
///
/// # `show_cutscene(id: String)`
/// Causes the cutscene with the specified `id` to show.  This blocks the user interface
/// until the cutscene is complete or the player skips it.  The cutscene is launched
/// asynchronously on the next frame, so the remaineder of this script script will execute
/// immediately.
///
/// # `scroll_view(x: Int, y: Int)`
/// Causes the view of the current area to scroll to the specified `x`, `y` coordinates.
/// This done using a smooth scroll effect.  The scroll begins on the next frame, so the
/// remainder of the current script will continue to execute immediately.
///
/// # `num_effects_with_tag(tag: String) -> Int`
/// Returns the number of currently active effects, in any area, with the specified effect
/// tag.  This can be used in scripts to enforce a global limit on a specific effect type.
///
/// # `has_party_member(id: String) -> Bool`
/// Returns true if one of the current party members has the specified `id`, false otherwise
///
/// # `add_party_member(id: String, show_portrait: Bool (Optional))`
/// Searches for an entity with the specified `id`.  If it is found, adds that entity as a
/// member of the player's party, making them controllable by the player.  If an entity with
/// the `id` is not found, throws an error.
/// `show_portrait` controls where the entity is displayed in the portraits area of the UI.  If
/// not passed, defaults to true.
///
/// # `remove_party_member(id: String)`
/// Removes the entity with the specified ID from the party, if it is currently in the party.
/// Does nothing otherwise.
///
/// # `party_coins() -> Int`
/// Returns the current amount of party coins.  Note that this value must be divided by the
/// item_value_display_factor in the module rules in order to get the displayed amount of
/// coins.
///
/// # `add_party_coins(amount: Int)`
/// Adds the specified number of coins to the party.  Note that this value is divided by
/// the item_value_display_factor to get the displayed coinage.
///
/// # `find_party_item(id: String, adjective: String (Optional, up to 3)) -> ScriptStashItem`
/// Returns a ScriptStashItem representing the first item in the party stash found
/// matching the specified ID and all specified `adjective`s.  If no such item is found,
/// returns an invalid ScriptStashItem.
///
/// # `remove_party_item(item: ScriptStashItem)`
/// Removes a quantity of one of the specified item from the party stash.
///
/// # `add_party_item(id: String, adjective: String (Optional, up to 3)) -> ScriptStashItem`
/// Creates an item with the specified `id`, and `adjective`, if specified.  If there is
/// no item definition with this ID or the adjective is specified but there is no
/// adjective with that ID, throws an error.  Otherwise, the item is added to the party
/// stash.  Returns a `ScriptStashItem` representing the added item.
///
/// # `add_party_xp(amount: Int)`
/// Adds the specified amount of XP to the party.  Each current party member is given
/// this amount of XP.
///
/// # `transition_party_to(x: Int, y: Int, area: String (Optional))`
/// Moves the party to the specified coordinates within the specified area.  If an area is not
/// specified, the transition occurs within the current area.  If the area
/// or coordinates are invalid, this will currently leave the game in a bad state where
/// the player is forced to load to continue.  The player is moved to the exact coordinates,
/// whereas other party members are moved to nearby coordinates.
///
/// # `start_bench(tag: String (Optional)) -> Handle`
/// Starts a benchmark run.  Returns a `handle` that can be used to finish the run.
/// The benchmark may optionally be labeled with a `tag`.
///
/// # `end_bench(handle: Handle)`
/// Ends a benchmark run.  The `handle` should be the one returned from `start_bench`.
///
pub struct ScriptInterface {}

impl UserData for ScriptInterface {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("play_sfx", |_, _, (id, vol): (String, Option<f32>)| {
            let vol = vol.unwrap_or(1.0);
            sulis_core::io::Audio::play_sfx(&id, vol);
            Ok(())
        });

        methods.add_method("current_round", |_, _, ()| {
            let mgr = GameState::turn_manager();
            let round = mgr.borrow().current_round();
            Ok(round)
        });

        methods.add_method(
            "add_time",
            |_, _, (day, hour, round): (u32, Option<u32>, Option<u32>)| {
                let hour = hour.unwrap_or(0);
                let round = round.unwrap_or(0);
                let time = Time {
                    day,
                    hour,
                    round,
                    millis: 0,
                };

                let mgr = GameState::turn_manager();
                mgr.borrow_mut().add_time(time);
                Ok(())
            },
        );

        methods.add_method("current_time", |lua, _, ()| {
            let mgr = GameState::turn_manager();
            let time = mgr.borrow().current_time();
            let table = lua.create_table()?;
            table.set("day", time.day)?;
            table.set("hour", time.hour)?;
            table.set("round", time.round)?;
            Ok(table)
        });

        methods.add_method("party", |lua, _, ()| {
            let table = lua.create_table()?;
            for (index, member) in GameState::party().iter().enumerate() {
                table.set(index + 1, ScriptEntity::from(member))?;
            }
            Ok(table)
        });

        methods.add_method("has_targeter", |_, _, ()| {
            let area_state = GameState::area_state();
            let area_state = area_state.borrow();
            Ok(area_state.targeter().is_some())
        });

        methods.add_method("check_targeter_position", |_, _, (x, y): (i32, i32)| {
            let targeter = get_targeter()?;
            let mut targeter = targeter.borrow_mut();
            targeter.on_mouse_move(x, y);

            let result = targeter.is_valid_to_activate();
            Ok(result)
        });

        methods.add_method("get_targeter_affected", |_, _, ()| {
            let targeter = get_targeter()?;
            let targeter = targeter.borrow();
            let parent = targeter.parent();
            let affected: Vec<_> = targeter
                .cur_affected()
                .iter()
                .map(|e| Some(Rc::clone(e)))
                .collect();
            Ok(ScriptEntitySet::new(&parent, &affected))
        });

        methods.add_method("get_targeter_selectable", |_, _, ()| {
            let targeter = get_targeter()?;
            let targeter = targeter.borrow();
            let parent = targeter.parent();
            let selectable: Vec<_> = targeter
                .selectable()
                .iter()
                .map(|e| Some(Rc::clone(e)))
                .collect();
            Ok(ScriptEntitySet::new(&parent, &selectable))
        });

        methods.add_method("is_targeter_free_select", |_, _, ()| {
            let targeter = get_targeter()?;
            let targeter = targeter.borrow();
            Ok(targeter.is_free_select())
        });

        methods.add_method("cancel_targeter", |_, _, ()| {
            let targeter = get_targeter()?;
            targeter.borrow_mut().on_cancel();
            Ok(())
        });

        methods.add_method("activate_targeter", |_, _, ()| {
            let targeter = get_targeter()?;
            targeter.borrow_mut().on_activate();
            Ok(())
        });

        methods.add_method("cancel_blocking_anims", |_, _, ()| {
            GameState::remove_all_blocking_animations();
            Ok(())
        });

        methods.add_method("check_ai_activation", |_, _, entity: ScriptEntity| {
            let entity = entity.try_unwrap()?;
            let area = GameState::get_area_state(&entity.borrow().location.area_id).unwrap();
            let mgr = GameState::turn_manager();
            mgr.borrow_mut()
                .check_ai_activation(&entity, &mut area.borrow_mut());
            Ok(())
        });

        methods.add_method("fade_out_in", |_, _, ()| {
            let pc = GameState::player();
            let cb = OnTrigger::FadeOutIn;
            GameState::add_ui_callback(vec![cb], &pc, &pc);
            Ok(())
        });

        methods.add_method("init_party_day", |_, _, ()| {
            for member in GameState::party() {
                member.borrow_mut().actor.init_day();
            }
            Ok(())
        });

        methods.add_method("create_menu_selection", |_, _, value: String| {
            let out = ScriptMenuSelection { value };
            Ok(out)
        });

        methods.add_method(
            "create_menu",
            |_, _, (title, cb): (String, CallbackData)| {
                let menu = ScriptMenu::new(title, cb);
                Ok(menu)
            },
        );

        methods.add_method(
            "show_confirm",
            |_, _, (msg, accept, cancel, id, func): (String, String, String, String, String)| {
                let pc = GameState::player();
                let data = on_trigger::DialogData {
                    message: msg,
                    accept_text: accept,
                    cancel_text: cancel,
                    on_accept: Some(on_trigger::ScriptData { id, func }),
                };
                let cb = OnTrigger::ShowConfirm(data);
                GameState::add_ui_callback(vec![cb], &pc, &pc);
                Ok(())
            },
        );

        methods.add_method("warn", |_, _, val: String| {
            warn!("[LUA WARN]: {}", val);
            Ok(())
        });

        methods.add_method("log", |_, _, val: String| {
            info!("[LUA]: {}", val);
            Ok(())
        });

        methods.add_method("debug", |_, _, val: String| {
            debug!("[LUA]: {}", val);
            Ok(())
        });

        methods.add_method("trace", |_, _, val: String| {
            debug!("[LUA]: {}", val);
            Ok(())
        });

        methods.add_method("ap_display_factor", |_, _, ()| {
            let rules = Module::rules();
            Ok(rules.display_ap)
        });

        methods.add_method("anim_base_time", |_, _, ()| {
            let secs = Config::animation_base_time_millis() as f32 / 1000.0;
            Ok(secs)
        });

        methods.add_method("atan2", |_, _, (x, y): (f32, f32)| Ok(y.atan2(x)));

        methods.add_method("block_ui", |_, _, time: f32| {
            let pc = GameState::player();
            let cb = OnTrigger::BlockUI((time * 1000.0) as u32);
            GameState::add_ui_callback(vec![cb], &pc, &pc);
            Ok(())
        });

        methods.add_method(
            "run_script_delayed",
            |_, _, (script, func, delay): (String, String, f32)| {
                let player = GameState::player();
                let parent = player.borrow().index();
                let mut cb_data = CallbackData::new_trigger(parent, script);
                cb_data.add_func(FuncKind::OnAnimComplete, func);

                let mut anim = Anim::new_non_blocking_wait(&player, (delay * 1000.0) as u32);
                anim.add_completion_callback(Box::new(cb_data));

                GameState::add_animation(anim);
                Ok(())
            },
        );

        methods.add_method(
            "create_callback",
            |_, _, (parent, script): (ScriptEntity, String)| {
                let index = parent.try_unwrap_index()?;
                let cb_data = CallbackData::new_trigger(index, script);
                Ok(cb_data)
            },
        );

        methods.add_method(
            "set_quest_state",
            |_, _, (quest, state): (String, String)| {
                let state = QuestEntryState::unwrap_from_str(&state);
                if Module::quest(&quest).is_none() {
                    warn!("Set quest state for invalid quest '{}'", quest);
                }
                GameState::set_quest_state(quest, state);
                Ok(())
            },
        );

        methods.add_method(
            "set_quest_entry_state",
            |_, _, (quest, entry, state): (String, String, String)| {
                let state = QuestEntryState::unwrap_from_str(&state);
                match Module::quest(&quest) {
                    None => warn!("Set quest entry state for invalid quest '{}'", quest),
                    Some(ref quest) => {
                        if !quest.entries.contains_key(&entry) {
                            warn!(
                                "Set quest entry state for invalid entry '{}' in '{:?}'",
                                entry, quest
                            );
                        }
                    }
                }

                GameState::set_quest_entry_state(quest, entry, state);
                Ok(())
            },
        );

        methods.add_method("get_quest_state", |_, _, quest: String| {
            if Module::quest(&quest).is_none() {
                warn!("Requested state for invalid quest '{}'", quest);
            }
            Ok(format!("{:?}", GameState::get_quest_state(quest)))
        });

        methods.add_method(
            "get_quest_entry_state",
            |_, _, (quest, entry): (String, String)| {
                match Module::quest(&quest) {
                    None => warn!("Requested entry state for invalid quest '{}'", quest),
                    Some(ref quest) => {
                        if !quest.entries.contains_key(&entry) {
                            warn!(
                                "Requested entry state for invalid entry '{}' in '{:?}'",
                                entry, quest
                            );
                        }
                    }
                }
                Ok(format!(
                    "{:?}",
                    GameState::get_quest_entry_state(quest, entry)
                ))
            },
        );

        methods.add_method(
            "set_world_map_location_visible",
            |_, _, (location, vis): (String, bool)| {
                GameState::set_world_map_location_visible(&location, vis);
                Ok(())
            },
        );

        methods.add_method(
            "set_world_map_location_enabled",
            |_, _, (location, en): (String, bool)| {
                GameState::set_world_map_location_enabled(&location, en);
                Ok(())
            },
        );

        methods.add_method(
            "is_passable",
            |_, _, (entity, x, y): (ScriptEntity, i32, i32)| {
                let area_state = GameState::area_state();
                let area_state = area_state.borrow();
                let entity = entity.try_unwrap()?;
                let entity = entity.borrow();
                let entities_to_ignore = vec![entity.index()];
                Ok(area_state.is_passable(&entity, &entities_to_ignore, x, y))
            },
        );

        methods.add_method(
            "spawn_actor_at",
            |_, _, (id, x, y, faction): (String, i32, i32, Option<String>)| {
                let actor = match Module::actor(&id) {
                    None => {
                        warn!("Unable to spawn actor '{}': not found", id);
                        return Ok(ScriptEntity::invalid());
                    }
                    Some(actor) => actor,
                };

                let size = Rc::clone(&actor.race.size);

                let area_state = GameState::area_state();

                if !area_state.borrow().is_passable_size(&size, x, y) {
                    info!(
                        "Unable to spawn actor '{}' at {},{}: not passable",
                        id, x, y
                    );
                    return Ok(ScriptEntity::invalid());
                }

                let location = Location::new(x, y, &area_state.borrow().area.area);
                let result = match area_state
                    .borrow_mut()
                    .add_actor(actor, location, None, false, None)
                {
                    Ok(index) => ScriptEntity::new(index),
                    Err(e) => {
                        warn!("Error spawning actor in area: {}", e);
                        return Ok(ScriptEntity::invalid());
                    }
                };

                let entity = result.try_unwrap()?;
                if let Some(faction) = faction {
                    match Faction::option_from_str(&faction) {
                        None => warn!("Invalid faction '{}' in script", faction),
                        Some(faction) => entity.borrow_mut().actor.set_faction(faction),
                    }
                }

                let mgr = GameState::turn_manager();
                mgr.borrow_mut()
                    .check_ai_activation(&entity, &mut area_state.borrow_mut());
                mgr.borrow_mut()
                    .check_ai_activation_for_party(&mut area_state.borrow_mut());

                Ok(result)
            },
        );

        methods.add_method(
            "spawn_encounter_at",
            |_, _, (x, y, id): (i32, i32, Option<String>)| {
                let area_state = get_area(id)?;
                let mut area_state = area_state.borrow_mut();

                if !area_state.spawn_encounter_at(x, y) {
                    warn!("Unable to find encounter for script spawn at {},{}", x, y);
                }

                let mgr = GameState::turn_manager();
                mgr.borrow_mut()
                    .check_ai_activation_for_party(&mut area_state);

                Ok(())
            },
        );

        methods.add_method(
            "enable_trigger_at",
            |_, _, (x, y, id): (i32, i32, Option<String>)| {
                let area_state = get_area(id)?;
                let mut area_state = area_state.borrow_mut();
                if !area_state.set_trigger_enabled_at(x, y, true) {
                    warn!("Unable to find trigger at {},{}", x, y);
                }
                Ok(())
            },
        );

        methods.add_method(
            "disable_trigger_at",
            |_, _, (x, y, id): (i32, i32, Option<String>)| {
                let area_state = get_area(id)?;
                let mut area_state = area_state.borrow_mut();
                if !area_state.set_trigger_enabled_at(x, y, false) {
                    warn!("Unable to find trigger at {},{}", x, y);
                }
                Ok(())
            },
        );

        methods.add_method(
            "enable_prop_at",
            |_, _, (x, y, id): (i32, i32, Option<String>)| {
                let area_state = get_area(id)?;
                let mut area_state = area_state.borrow_mut();
                if !area_state.props_mut().set_enabled_at(x, y, true) {
                    warn!("Unable to find prop at {},{}", x, y);
                }
                Ok(())
            },
        );

        methods.add_method(
            "disable_prop_at",
            |_, _, (x, y, id): (i32, i32, Option<String>)| {
                let area_state = get_area(id)?;
                let mut area_state = area_state.borrow_mut();
                if !area_state.props_mut().set_enabled_at(x, y, false) {
                    warn!("Unable to find prop at {},{}", x, y);
                }
                Ok(())
            },
        );

        methods.add_method(
            "toggle_prop_at",
            |_, _, (x, y, id): (i32, i32, Option<String>)| {
                let area_state = get_area(id)?;
                let mut area_state = area_state.borrow_mut();
                let index = match area_state.props().index_at(x, y) {
                    None => {
                        warn!("Unable to find prop at {},{}", x, y);
                        return Ok(());
                    }
                    Some(prop) => prop,
                };
                area_state.toggle_prop_active(index);

                Ok(())
            },
        );

        methods.add_method(
            "say_line",
            |_, _, (line, target): (String, Option<ScriptEntity>)| {
                let pc = GameState::player();
                let target = match target {
                    None => Rc::clone(&pc),
                    Some(ref entity) => entity.try_unwrap()?,
                };

                let cb = OnTrigger::SayLine(line);
                GameState::add_ui_callback(vec![cb], &pc, &target);
                Ok(())
            },
        );

        methods.add_method(
            "start_conversation",
            |_, _, (id, target): (String, Option<ScriptEntity>)| {
                let pc = GameState::player();
                let target = match target {
                    None => Rc::clone(&pc),
                    Some(ref entity) => entity.try_unwrap()?,
                };

                let cb = OnTrigger::StartConversation(id);
                GameState::add_ui_callback(vec![cb], &pc, &target);
                Ok(())
            },
        );

        methods.add_method("show_game_over_window", |_, _, text: String| {
            let pc = GameState::player();
            let cb = OnTrigger::GameOverWindow(text);
            GameState::add_ui_callback(vec![cb], &pc, &pc);
            Ok(())
        });

        methods.add_method("create_module_export", |_, _, id: String| {
            Ok(ModuleExport::new(id))
        });

        methods.add_method("player", |_, _, ()| {
            Ok(ScriptEntity::from(&GameState::player()))
        });

        methods.add_method("show_cutscene", |_, _, id: String| {
            let pc = GameState::player();
            let cb = OnTrigger::ShowCutscene(id);
            GameState::add_ui_callback(vec![cb], &pc, &pc);
            Ok(())
        });

        methods.add_method("scroll_view", |_, _, (x, y): (i32, i32)| {
            let pc = GameState::player();
            let cb = OnTrigger::ScrollView(x, y);
            GameState::add_ui_callback(vec![cb], &pc, &pc);
            Ok(())
        });

        methods.add_method("num_effects_with_tag", |_, _, tag: String| {
            let mgr = GameState::turn_manager();
            let mgr = mgr.borrow();

            let mut count = 0;
            for effect in mgr.effect_iter() {
                if effect.tag == tag {
                    count += 1;
                }
            }
            Ok(count)
        });

        methods.add_method("entities_with_ids", |_, _, ids: Vec<String>| {
            Ok(entities_with_ids(ids))
        });

        methods.add_method("entity_with_id", |_, _, id: String| {
            match entity_with_id(id) {
                Some(entity) => Ok(ScriptEntity::from(&entity)),
                None => Ok(ScriptEntity::invalid()),
            }
        });

        methods.add_method("has_party_member", |_, _, id: String| {
            Ok(GameState::has_party_member(&id))
        });

        methods.add_method(
            "add_party_member",
            |_, _, (id, show_portrait): (String, Option<bool>)| {
                let show_portrait = show_portrait.unwrap_or(true);
                let entity = match entity_with_id(id) {
                    Some(entity) => entity,
                    None => {
                        return Err(rlua::Error::ToLuaConversionError {
                            from: "ID",
                            to: "ScriptENtity",
                            message: Some("Entity with specified id does not exist".to_string()),
                        });
                    }
                };

                GameState::add_party_member(entity, show_portrait);

                Ok(())
            },
        );

        methods.add_method("remove_party_member", |_, _, id: String| {
            for member in GameState::party() {
                if member.borrow().unique_id() == id {
                    GameState::remove_party_member(member);
                    break;
                }
            }
            Ok(())
        });

        methods.add_method("party_coins", |_, _, ()| {
            let coins = GameState::party_coins();
            Ok(coins)
        });

        methods.add_method("add_party_coins", |_, _, amount: i32| {
            GameState::add_party_coins(amount);
            let stash = GameState::party_stash();
            let stash = &stash.borrow();
            stash.listeners.notify(stash);

            let factor = Module::rules().item_value_display_factor;
            let pc = GameState::player();
            let line = if amount >= 0 {
                format!("Gained {} coins", (amount as f32 / factor) as i32)
            } else {
                format!("Lost {} coins", (-amount as f32 / factor) as i32)
            };

            let cb = OnTrigger::SayLine(line);
            GameState::add_ui_callback(vec![cb], &pc, &pc);

            Ok(())
        });

        methods.add_method("find_party_item", |_, _, (id, adj1, adj2, adj3):
                           (String, Option<String>, Option<String>, Option<String>)| {

            let adjs = vec![adj1, adj2, adj3];
            let mut identified = true;
            let adjectives: Vec<_> = adjs.into_iter().filter_map(|a| {
                if a.as_ref() == Some(&Module::rules().unidentified_item_adjective) {
                    identified = false;
                    None
                } else {
                    a
                }
            }).collect();
            let stash = GameState::party_stash();
            let item = match Module::create_get_item(&id, &adjectives) {
                None => return Err(rlua::Error::FromLuaConversionError {
                    from: "String",
                    to: "Item",
                    message: Some(format!("Item '{}' does not exist", id)),
                }),
                Some(item) => item,
            };
            let item_state = ItemState::new(item, None, identified);
            let index = stash.borrow().items().find_index(&item_state);
            Ok(ScriptStashItem { index })
        });

        methods.add_method("remove_party_item", |_, _, item: ScriptStashItem| {
            let stash = GameState::party_stash();
            if let Some(index) = item.index {
                // throw away item
                let _ = stash.borrow_mut().remove_item(index);
            }
            Ok(())
        });

        methods.add_method("add_party_item", |_, _, (item, adj1, adj2, adj3):
            (String, Option<String>, Option<String>, Option<String>)| {
            let adjs = vec![adj1, adj2, adj3];
            let mut identified = true;
            let adjectives: Vec<_> = adjs.into_iter().filter_map(|a| {
                if a.as_ref() == Some(&Module::rules().unidentified_item_adjective) {
                    identified = false;
                    None
                } else {
                    a
                }
            }).collect();

            let stash = GameState::party_stash();
            let item = match Module::create_get_item(&item, &adjectives) {
                None => return Err(rlua::Error::FromLuaConversionError {
                    from: "String",
                    to: "Item",
                    message: Some(format!("Item '{}' does not exist", item)),
                }),
                Some(item) => item,
            };

            let item_state = ItemState::new(item, None, identified);
            let index = stash.borrow_mut().add_item(1, item_state);
            Ok(ScriptStashItem { index })
        });

        methods.add_method("add_party_xp", |_, _, amount: u32| {
            for member in GameState::party().iter() {
                member.borrow_mut().add_xp(amount);
            }

            let pc = GameState::player();
            let line = format!("Gained {} xp", amount);
            let cb = OnTrigger::SayLine(line);
            GameState::add_ui_callback(vec![cb], &pc, &pc);
            Ok(())
        });

        methods.add_method(
            "transition_party_to",
            |_, _, (x, y, area): (i32, i32, Option<String>)| {
                let id = area.as_ref().map(|s| &**s);
                let p = Some(Point::new(x, y));
                let time = Time {
                    day: 0,
                    hour: 0,
                    round: 0,
                    millis: 0,
                };
                GameState::transition_to(id, p, Point::default(), time);
                Ok(())
            },
        );

        methods.add_method("start_bench", |_, _, tag: Option<String>| {
            let handle = sulis_core::benchmark::start_bench(tag);

            Ok(handle)
        });

        methods.add_method("end_bench", |_, _, handle: sulis_core::benchmark::Handle| {
            sulis_core::benchmark::end_bench(handle);
            Ok(())
        });
    }
}

fn get_area(id: Option<String>) -> Result<Rc<RefCell<AreaState>>> {
    match id {
        None => Ok(GameState::area_state()),
        Some(id) => GameState::get_area_state(&id).ok_or(rlua::Error::FromLuaConversionError {
            from: "String",
            to: "AreaState",
            message: Some(format!(
                "The area '{}' does not exist or is not loaded.",
                id
            )),
        }),
    }
}

fn entities_with_ids(ids: Vec<String>) -> Vec<ScriptEntity> {
    let mut result = Vec::new();

    let mgr = GameState::turn_manager();
    for id in ids {
        for entity in mgr.borrow().entity_iter() {
            if entity.borrow().unique_id() == id {
                result.push(ScriptEntity::from(&entity));
                break;
            }
        }
    }

    result
}

pub fn entity_with_id(id: String) -> Option<Rc<RefCell<EntityState>>> {
    let mgr = GameState::turn_manager();
    for entity in mgr.borrow().entity_iter() {
        {
            let entity = entity.borrow();
            if entity.unique_id() != id {
                continue;
            }
        }

        return Some(entity);
    }

    None
}
