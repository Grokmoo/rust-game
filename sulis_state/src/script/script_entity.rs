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

use std::{self, f32, u32};
use std::rc::Rc;
use std::cell::RefCell;
use std::collections::HashMap;

use rlua::{self, Lua, UserData, UserDataMethods};

use animation::{Animation, MeleeAttackAnimation};
use sulis_rules::{AttackKind, DamageKind, Attack};
use sulis_core::config::CONFIG;
use sulis_core::ui::color;
use {ActorState, EntityState, GameState};
use script::*;

#[derive(Clone, Debug)]
pub struct ScriptEntity {
    pub index: Option<usize>,
}

impl ScriptEntity {
    pub fn new(index: usize) -> ScriptEntity {
        ScriptEntity { index: Some(index) }
    }

    pub fn from(entity: &Rc<RefCell<EntityState>>) -> ScriptEntity {
        ScriptEntity { index: Some(entity.borrow().index) }
    }

    pub fn check_not_equal(&self, other: &ScriptEntity) -> Result<()> {
        if self.index == other.index {
            warn!("Parent and target must not refer to the same entity for this method");
            Err(rlua::Error::FromLuaConversionError {
                from: "ScriptEntity",
                to: "ScriptEntity",
                message: Some("Parent and target must not match".to_string())
            })
        } else {
            Ok(())
        }
    }

    pub fn try_unwrap_index(&self) -> Result<usize> {
        match self.index {
            None => Err(rlua::Error::FromLuaConversionError {
                from: "ScriptEntity",
                to: "EntityState",
                message: Some("ScriptEntity does not have a valid index".to_string())
            }),
            Some(index) => Ok(index),
        }
    }

    pub fn try_unwrap(&self) -> Result<Rc<RefCell<EntityState>>> {
        match self.index {
            None => Err(rlua::Error::FromLuaConversionError {
                from: "ScriptEntity",
                to: "EntityState",
                message: Some("ScriptEntity does not have a valid index".to_string())
            }),
            Some(index) => {
                let area_state = GameState::area_state();
                let area_state = area_state.borrow();
                match area_state.check_get_entity(index) {
                    None => Err(rlua::Error::FromLuaConversionError {
                        from: "ScriptEntity",
                        to: "EntityState",
                        message: Some("ScriptEntity refers to an entity that no longer exists.".to_string())
                    }),
                    Some(entity) => Ok(entity),
                }
            }
        }
    }
}

impl UserData for ScriptEntity {
    fn add_methods(methods: &mut UserDataMethods<Self>) {
        methods.add_method("set_flag", |_, entity, flag: String| {
            let entity = entity.try_unwrap()?;
            entity.borrow_mut().set_custom_flag(&flag);
            Ok(())
        });

        methods.add_method("is_valid", |_, entity, ()| {
            let area_state = GameState::area_state();
            match entity.index {
                None => Ok(false),
                Some(index) => Ok(area_state.borrow().has_entity(index)),
            }
        });

        methods.add_method("targets", &targets);

        methods.add_method("create_effect", |_, entity, args: (String, Option<u32>)| {
            let duration = args.1.unwrap_or(u32::MAX);
            let ability = args.0;
            let index = entity.try_unwrap_index()?;
            Ok(ScriptEffect::new(index, &ability, duration))
        });

        methods.add_method("create_subpos_anim", |_, entity, duration_secs: f32| {
            let index = entity.try_unwrap_index()?;
            Ok(ScriptSubposAnimation::new(index, duration_secs))
        });

        methods.add_method("create_color_anim", |_, entity, duration_secs: Option<f32>| {
            let index = entity.try_unwrap_index()?;
            let duration = duration_secs.unwrap_or(f32::INFINITY);
            Ok(ScriptColorAnimation::new(index, duration))
        });

        methods.add_method("create_particle_generator", |_, entity, args: (String, Option<f32>)| {
            let duration_secs = args.1.unwrap_or(f32::INFINITY);
            let sprite = args.0;
            let index = entity.try_unwrap_index()?;
            Ok(ScriptParticleGenerator::new(index, sprite, duration_secs))
        });

        methods.add_method("create_anim", |_, entity, (image, duration): (String, Option<f32>)| {
            let duration = duration.unwrap_or(f32::INFINITY);
            let index = entity.try_unwrap_index()?;
            Ok(ScriptParticleGenerator::new_anim(index, image, duration))
        });

        methods.add_method("create_targeter", |_, entity, ability: ScriptAbility| {
            let index = entity.try_unwrap_index()?;
            Ok(TargeterData::new(index, &ability.id))
        });

        methods.add_method("teleport_to", |_, entity, dest: HashMap<String, i32>| {
            let (x, y) = unwrap_point(dest)?;
            let entity = entity.try_unwrap()?;

            let area_state = GameState::area_state();
            let mut area_state = area_state.borrow_mut();
            area_state.move_entity(&entity, x, y, 0);
            Ok(())
        });

        methods.add_method("weapon_attack", |_, entity, target: ScriptEntity| {
            let target = target.try_unwrap()?;
            let parent = entity.try_unwrap()?;
            let (hit_kind, damage, text, color) = ActorState::weapon_attack(&parent, &target);

            let area_state = GameState::area_state();
            area_state.borrow_mut().add_feedback_text(text, &target, color, 3.0);

            let hit_kind = ScriptHitKind { kind: hit_kind, damage };
            Ok(hit_kind)
        });

        methods.add_method("anim_weapon_attack", |_, entity, (target, callback):
                           (ScriptEntity, Option<CallbackData>)| {
            entity.check_not_equal(&target)?;
            let parent = entity.try_unwrap()?;
            let target = target.try_unwrap()?;

            let cb: Option<Box<ScriptCallback>> = match callback {
                None => None,
                Some(cb) => Some(Box::new(cb)),
            };

            EntityState::attack(&parent, &target, cb);
            Ok(())
        });

        methods.add_method("anim_special_attack", |_, entity,
                           (target, attack_kind, min_damage, max_damage, ap, damage_kind, cb):
                           (ScriptEntity, String, u32, u32, u32, String, Option<CallbackData>)| {
            entity.check_not_equal(&target)?;
            let parent = entity.try_unwrap()?;
            let target = target.try_unwrap()?;
            let damage_kind = DamageKind::from_str(&damage_kind);
            let attack_kind = AttackKind::from_str(&attack_kind);

            let time = CONFIG.display.animation_base_time_millis * 5;
            let mut anim = MeleeAttackAnimation::new(&parent, &target, time, Box::new(move |att, def| {
                let attack = Attack::special(min_damage, max_damage, ap,
                                             damage_kind, attack_kind.clone());

                ActorState::attack(att, def, &attack)
            }));

            if let Some(cb) = cb {
                anim.set_callback(Some(Box::new(cb)));
            }
            GameState::add_animation(Box::new(anim));
            Ok(())
        });

        methods.add_method("special_attack", |_, entity,
                           (target, attack_kind, min_damage, max_damage, ap, damage_kind):
                           (ScriptEntity, String, Option<u32>, Option<u32>, Option<u32>, Option<String>)| {
            let target = target.try_unwrap()?;
            let parent = entity.try_unwrap()?;

            let damage_kind = match damage_kind {
                None => DamageKind::Raw,
                Some(ref kind) => DamageKind::from_str(kind),
            };
            let attack_kind = AttackKind::from_str(&attack_kind);

            let min_damage = min_damage.unwrap_or(0);
            let max_damage = max_damage.unwrap_or(0);
            let ap = ap.unwrap_or(0);

            let attack = Attack::special(min_damage, max_damage, ap, damage_kind, attack_kind);

            let (hit_kind, damage, text, color) = ActorState::attack(&parent, &target, &attack);

            let area_state = GameState::area_state();
            area_state.borrow_mut().add_feedback_text(text, &target, color, 3.0);

            let hit_kind = ScriptHitKind { kind: hit_kind, damage };
            Ok(hit_kind)
        });

        methods.add_method("take_damage", |_, entity, (min_damage, max_damage, damage_kind, ap):
                           (u32, u32, String, Option<u32>)| {
            let parent = entity.try_unwrap()?;
            let damage_kind = DamageKind::from_str(&damage_kind);

            let attack = Attack::special(min_damage, max_damage,
                                         ap.unwrap_or(0), damage_kind, AttackKind::Fortitude);
            let damage = attack.roll_damage(&parent.borrow().actor.stats.armor, 1.0);

            let (text, color) = if !damage.is_empty() {
                let mut total = 0;
                for (_, amount) in damage {
                    total += amount;
                }

                parent.borrow_mut().remove_hp(total);
                (format!("{}", total), color::RED)
            } else {
                ("0".to_string(), color::GRAY)
            };

            let area_state = GameState::area_state();
            area_state.borrow_mut().add_feedback_text(text, &parent, color, 3.0);
            Ok(())
        });

        methods.add_method("heal_damage", |_, entity, amount: u32| {
            let parent = entity.try_unwrap()?;
            parent.borrow_mut().actor.add_hp(amount);
            let area_state = GameState::area_state();
            area_state.borrow_mut().add_feedback_text(format!("{}", amount), &parent, color::GREEN, 3.0);

            Ok(())
        });

        methods.add_method("change_overflow_ap", |_, entity, ap| {
            let entity = entity.try_unwrap()?;
            entity.borrow_mut().actor.change_overflow_ap(ap);
            Ok(())
        });

        methods.add_method("set_subpos", |_, entity, (x, y): (f32, f32)| {
            let entity = entity.try_unwrap()?;
            entity.borrow_mut().sub_pos = (x, y);
            Ok(())
        });

        methods.add_method("remove_ap", |_, entity, ap| {
            let entity = entity.try_unwrap()?;
            entity.borrow_mut().actor.remove_ap(ap);
            Ok(())
        });

        methods.add_method("name", |_, entity, ()| {
            let entity = entity.try_unwrap()?;
            let entity = entity.borrow();
            Ok(entity.actor.actor.name.to_string())
        });

        methods.add_method("has_ability", |_, entity, id: String| {
            let entity = entity.try_unwrap()?;
            let entity = entity.borrow();
            Ok(entity.actor.actor.has_ability_with_id(&id))
        });

        methods.add_method("has_active_mode", |_, entity, ()| {
            let entity = entity.try_unwrap()?;
            let entity = entity.borrow();
            for (_, ref state) in entity.actor.ability_states.iter() {
                if state.is_active_mode() { return Ok(true); }
            }
            Ok(false)
        });

        methods.add_method("stats", &create_stats_table);

        methods.add_method("inventory", |_, entity, ()| {
            Ok(ScriptInventory::new(entity.clone()))
        });

        methods.add_method("size_str", |_, entity, ()| {
            let entity = entity.try_unwrap()?;
            let entity = entity.borrow();
            Ok(entity.size().to_string())
        });
        methods.add_method("width", |_, entity, ()| {
            let entity = entity.try_unwrap()?;
            let entity = entity.borrow();
            Ok(entity.size.width)
        });
        methods.add_method("height", |_, entity, ()| {
            let entity = entity.try_unwrap()?;
            let entity = entity.borrow();
            Ok(entity.size.height)
        });
        methods.add_method("x", |_, entity, ()| {
            let entity = entity.try_unwrap()?;
            let x = entity.borrow().location.x;
            Ok(x)
        });
        methods.add_method("y", |_, entity, ()| {
            let entity = entity.try_unwrap()?;
            let y = entity.borrow().location.y;
            Ok(y)
        });
        methods.add_method("center_x", |_, entity, ()| {
            let entity = entity.try_unwrap()?;
            let x = entity.borrow().location.x as f32 + entity.borrow().size.width as f32 / 2.0;
            Ok(x)
        });

        methods.add_method("center_y", |_, entity, ()| {
            let entity = entity.try_unwrap()?;
            let y = entity.borrow().location.y as f32 + entity.borrow().size.height as f32 / 2.0;
            Ok(y)
        });

        methods.add_method("dist_to_entity", |_, entity, target: ScriptEntity| {
            let entity = entity.try_unwrap()?;
            let target = target.try_unwrap()?;
            let entity = entity.borrow();
            Ok(entity.dist_to_entity(&target))
        });

        methods.add_method("dist_to_point", |_, entity, point: HashMap<String, i32>| {
            let (x, y) = unwrap_point(point)?;
            let entity = entity.try_unwrap()?;
            let entity = entity.borrow();
            Ok(entity.dist_to_point(Point::new(x, y)))
        });
    }
}

pub fn unwrap_point(point: HashMap<String, i32>) -> Result<(i32, i32)> {
    let x = match point.get("x") {
        None => return Err(rlua::Error::FromLuaConversionError {
            from: "ScriptPoint",
            to: "Point",
            message: Some("Point must have x and y coordinates".to_string())
        }),
        Some(x) => *x,
    };

    let y = match point.get("y") {
        None => return Err(rlua::Error::FromLuaConversionError {
            from: "ScriptPoint",
            to: "Point",
            message: Some("Point must have x and y coordinates".to_string())
        }),
        Some(y) => *y,
    };

    Ok((x, y))
}

fn create_stats_table<'a>(lua: &'a Lua, parent: &ScriptEntity, _args: ()) -> Result<rlua::Table<'a>> {
    let parent = parent.try_unwrap()?;
    let src = &parent.borrow().actor.stats;

    let stats = lua.create_table()?;
    stats.set("strength", src.attributes.strength)?;
    stats.set("dexterity", src.attributes.dexterity)?;
    stats.set("endurance", src.attributes.endurance)?;
    stats.set("perception", src.attributes.perception)?;
    stats.set("intellect", src.attributes.intellect)?;
    stats.set("wisdom", src.attributes.wisdom)?;

    stats.set("base_armor", src.armor.base())?;
    let armor = lua.create_table()?;
    for kind in DamageKind::iter() {
        armor.set(kind.to_str(), src.armor.amount(*kind))?;
    }
    stats.set("armor", armor)?;

    stats.set("bonus_reach", src.bonus_reach)?;
    stats.set("bonus_range", src.bonus_range)?;
    stats.set("max_hp", src.max_hp)?;
    stats.set("initiative", src.initiative)?;
    stats.set("accuracy", src.accuracy)?;
    stats.set("defense", src.defense)?;
    stats.set("fortitude", src.fortitude)?;
    stats.set("reflex", src.reflex)?;
    stats.set("will", src.will)?;

    stats.set("attack_distance", src.attack_distance() + parent.borrow().size.diagonal / 2.0)?;
    stats.set("attack_is_melee", src.attack_is_melee())?;
    stats.set("attack_is_ranged", src.attack_is_ranged())?;

    stats.set("concealment", src.concealment)?;
    stats.set("crit_threshold", src.crit_threshold)?;
    stats.set("graze_threshold", src.graze_threshold)?;
    stats.set("hit_threshold", src.hit_threshold)?;
    stats.set("graze_multiplier", src.graze_multiplier)?;
    stats.set("hit_multiplier", src.hit_multiplier)?;
    stats.set("crit_multiplier", src.crit_multiplier)?;
    stats.set("movement_rate", src.movement_rate)?;
    stats.set("attack_cost", src.attack_cost)?;

    if let Some(image) = src.get_ranged_projectile() {
        stats.set("ranged_projectile", image.id())?;
    }

    for (index, attack) in src.attacks.iter().enumerate() {
        stats.set(format!("damage_min_{}", index), attack.damage.min())?;
        stats.set(format!("damage_max_{}", index), attack.damage.max())?;
        stats.set(format!("armor_piercing_{}", index), attack.damage.ap())?;
    }

    Ok(stats)
}

#[derive(Clone, Debug)]
pub struct ScriptEntitySet {
    pub parent: usize,
    pub point: Option<(i32, i32)>,
    pub indices: Vec<Option<usize>>,
}

impl ScriptEntitySet {
    pub fn new(parent: &Rc<RefCell<EntityState>>,
               entities: &Vec<Option<Rc<RefCell<EntityState>>>>) -> ScriptEntitySet {
        let parent = parent.borrow().index;

        let indices = entities.iter().map(|e| {
            match e {
                &None => None,
                &Some(ref e) => Some(e.borrow().index),
            }
        }).collect();
        ScriptEntitySet {
            parent,
            point: None,
            indices,
        }
    }
}

impl UserData for ScriptEntitySet {
    fn add_methods(methods: &mut UserDataMethods<Self>) {
        methods.add_method("to_table", |_, set, ()| {
            let table: Vec<ScriptEntity> = set.indices.iter().map(|i| ScriptEntity { index: *i }).collect();

            Ok(table)
        });

        methods.add_method("selected_point", |_, set, ()| {
            match set.point {
                None => {
                    warn!("Attempted to get selected point from EntitySet where none is defined");
                    Err(rlua::Error::FromLuaConversionError {
                        from: "ScriptEntitySet",
                        to: "Point",
                        message: Some("EntitySet has no selected point".to_string())
                    })
                }, Some((x, y)) => {
                    let mut point = HashMap::new();
                    point.insert("x", x);
                    point.insert("y", y);
                    Ok(point)
                }
            }
        });
        methods.add_method("is_empty", |_, set, ()| Ok(set.indices.is_empty()));
        methods.add_method("first", |_, set, ()| {
            for index in set.indices.iter() {
                if let &Some(index) = index {
                    return Ok(ScriptEntity::new(index));
                }
            }

            warn!("Attempted to get first element of EntitySet that has no valid entities");
            Err(rlua::Error::FromLuaConversionError {
                from: "ScriptEntitySet",
                to: "ScriptEntity",
                message: Some("EntitySet is empty".to_string())
            })
        });

        methods.add_method("without_self", &without_self);
        methods.add_method("visible_within", &visible_within);
        methods.add_method("visible", |lua, set, ()| visible_within(lua, set, std::f32::MAX));
        methods.add_method("hostile", |lua, set, ()| is_hostile(lua, set));
        methods.add_method("friendly", |lua, set, ()| is_friendly(lua, set));
        methods.add_method("reachable", &reachable);
        methods.add_method("attackable", &attackable);
    }
}

fn targets(_lua: &Lua, parent: &ScriptEntity, _args: ()) -> Result<ScriptEntitySet> {
    let area_state = GameState::area_state();
    let area_state = area_state.borrow();

    let mut indices = Vec::new();
    for entity in area_state.entity_iter() {
        indices.push(Some(entity.borrow().index));
    }

    let parent_index = parent.try_unwrap_index()?;
    Ok(ScriptEntitySet { indices, parent: parent_index, point: None, })
}

fn without_self(_lua: &Lua, set: &ScriptEntitySet, _: ()) -> Result<ScriptEntitySet> {
    filter_entities(set, (), &|parent, entity, _| {
        !Rc::ptr_eq(parent, entity)
    })
}

fn visible_within(_lua: &Lua, set: &ScriptEntitySet, dist: f32) -> Result<ScriptEntitySet> {
    filter_entities(set, dist, &|parent, entity, dist| {
        if parent.borrow().dist_to_entity(entity) > dist { return false; }

        let area_state = GameState::area_state();
        let area_state = area_state.borrow();
        area_state.has_visibility(&parent.borrow(), &entity.borrow())
    })
}

fn attackable(_lua: &Lua, set: &ScriptEntitySet, _args: ()) -> Result<ScriptEntitySet> {
    filter_entities(set, (), &|parent, entity, _| {
        let area_state = GameState::area_state();
        let area_state = area_state.borrow();
        parent.borrow().can_attack(entity, &area_state)
    })
}

fn reachable(_lua: &Lua, set: &ScriptEntitySet, _args: ()) -> Result<ScriptEntitySet> {
    filter_entities(set, (), &|parent, entity, _| {
        parent.borrow().can_reach(entity)
    })
}

fn is_hostile(_lua: &Lua, set: &ScriptEntitySet) -> Result<ScriptEntitySet> {
    filter_entities(set, (), &|parent, entity, _| {
        parent.borrow().is_hostile(entity)
    })
}

fn is_friendly(_lua: &Lua, set: &ScriptEntitySet) -> Result<ScriptEntitySet> {
    filter_entities(set, (), &|parent, entity, _| {
        !parent.borrow().is_hostile(entity)
    })
}

fn filter_entities<T: Copy>(set: &ScriptEntitySet, t: T,
                  filter: &Fn(&Rc<RefCell<EntityState>>, &Rc<RefCell<EntityState>>, T) -> bool)
    -> Result<ScriptEntitySet> {

    let area_state = GameState::area_state();

    let parent = area_state.borrow().get_entity(set.parent);

    let mut indices = Vec::new();
    for index in set.indices.iter() {
        let entity = match index {
            &None => continue,
            &Some(index) => area_state.borrow().check_get_entity(index),
        };

        let entity = match entity {
            None => continue,
            Some(entity) => entity,
        };

        if !(filter)(&parent, &entity, t) { continue; }

        indices.push(*index);
    }

    Ok(ScriptEntitySet { indices, parent: set.parent, point: set.point, })
}
