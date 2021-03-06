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

use crate::{animation::Anim, entity_attack_handler::weapon_attack, AreaFeedbackText};
use crate::{script::ScriptEntitySet, EntityState, GameState, ScriptCallback};
use sulis_core::image::Image;
use sulis_core::io::{DrawList, GraphicsRenderer};
use sulis_core::ui::animation_state;
use sulis_core::util::{Offset, Rect, Scale};

pub(in crate::animation) fn update(
    attacker: &Rc<RefCell<EntityState>>,
    model: &mut RangedAttackAnimModel,
    frac: f32,
) {
    if frac > 1.0 {
        if !model.has_attacked {
            let cb_def_targets =
                ScriptEntitySet::new(&model.defender, &[Some(Rc::clone(attacker))]);
            let cb_att_targets =
                ScriptEntitySet::new(attacker, &[Some(Rc::clone(&model.defender))]);
            for cb in model.callbacks.iter() {
                cb.before_attack(&cb_def_targets);
            }

            let area_state = GameState::area_state();

            let (defender_cbs, attacker_cbs) = {
                let mgr = GameState::turn_manager();
                let mgr = mgr.borrow();
                (
                    model.defender.borrow().callbacks(&mgr),
                    attacker.borrow().callbacks(&mgr),
                )
            };

            attacker_cbs
                .iter()
                .for_each(|cb| cb.before_attack(&cb_att_targets));
            defender_cbs
                .iter()
                .for_each(|cb| cb.before_defense(&cb_def_targets));

            model.has_attacked = true;
            let result = weapon_attack(attacker, &model.defender);
            for entry in result {
                let (hit_kind, hit_flags, damage) = entry;

                let feedback = AreaFeedbackText::with_damage(
                    &model.defender.borrow(),
                    &area_state.borrow(),
                    hit_kind,
                    hit_flags,
                    &damage,
                );
                area_state.borrow_mut().add_feedback_text(feedback);

                for cb in model.callbacks.iter() {
                    cb.after_attack(&cb_def_targets, hit_kind, damage.clone());
                }

                attacker_cbs
                    .iter()
                    .for_each(|cb| cb.after_attack(&cb_att_targets, hit_kind, damage.clone()));
                defender_cbs
                    .iter()
                    .for_each(|cb| cb.after_defense(&cb_def_targets, hit_kind, damage.clone()));
            }
        }
    } else {
        model.cur_pos = (
            frac * model.vec.0 + model.start_pos.0,
            frac * model.vec.1 + model.start_pos.1,
        );
    }
}

pub(in crate::animation) fn cleanup(owner: &Rc<RefCell<EntityState>>) {
    if !GameState::is_combat_active() {
        let area_state = GameState::get_area_state(&owner.borrow().location.area_id).unwrap();
        let mgr = GameState::turn_manager();
        mgr.borrow_mut()
            .check_ai_activation(owner, &mut area_state.borrow_mut());
    }
}

pub(in crate::animation) fn draw(
    model: &RangedAttackAnimModel,
    renderer: &mut dyn GraphicsRenderer,
    offset: Offset,
    scale: Scale,
    millis: u32,
) {
    if let Some(ref projectile) = model.projectile {
        let rect = Rect {
            x: model.cur_pos.0 + offset.x,
            y: model.cur_pos.1 + offset.y,
            w: projectile.get_width_f32(),
            h: projectile.get_height_f32(),
        };

        let mut draw_list = DrawList::empty_sprite();
        projectile.append_to_draw_list(&mut draw_list, &animation_state::NORMAL, rect, millis);
        draw_list.set_scale(scale);
        draw_list.rotate(model.angle);
        renderer.draw(draw_list);
    }
}

pub fn new(
    attacker: &Rc<RefCell<EntityState>>,
    defender: &Rc<RefCell<EntityState>>,
    callbacks: Vec<Box<dyn ScriptCallback>>,
    duration_millis: u32,
) -> Anim {
    let mut start_pos = (
        (attacker.borrow().location.x + attacker.borrow().size.width / 2) as f32,
        (attacker.borrow().location.y + attacker.borrow().size.height / 2) as f32,
    );

    let x = (defender.borrow().location.x + defender.borrow().size.width / 2) as f32 - start_pos.0;
    let y = (defender.borrow().location.y + defender.borrow().size.height / 2) as f32 - start_pos.1;
    let dist = (x * x + y * y).sqrt();

    let projectile = attacker.borrow().actor.stats.get_ranged_projectile();
    if let Some(ref projectile) = projectile {
        start_pos.0 -= projectile.get_width_f32() / 2.0;
        start_pos.1 -= projectile.get_height_f32() / 2.0;
    }

    let angle = y.atan2(x);

    let model = RangedAttackAnimModel {
        defender: Rc::clone(defender),
        angle,
        vec: (x, y),
        start_pos,
        cur_pos: (0.0, 0.0),
        has_attacked: false,
        projectile,
        callbacks,
    };

    let millis = (duration_millis as f32 * dist) as u32;
    Anim::new_ranged_attack(attacker, millis, model)
}

pub(in crate::animation) struct RangedAttackAnimModel {
    defender: Rc<RefCell<EntityState>>,
    angle: f32,
    vec: (f32, f32),
    start_pos: (f32, f32),
    cur_pos: (f32, f32),
    pub(in crate::animation) has_attacked: bool,
    projectile: Option<Rc<dyn Image>>,
    callbacks: Vec<Box<dyn ScriptCallback>>,
}
