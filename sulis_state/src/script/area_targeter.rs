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
use std::f32::consts::PI;
use std::rc::Rc;

use sulis_core::image::Image;
use sulis_core::io::{DrawList, GraphicsRenderer};
use sulis_core::ui::{animation_state, color, Cursor, LineRenderer};
use sulis_core::util::{Offset, Point, Rect, Scale};
use sulis_module::{Ability, Module, ObjectSize};

use crate::script::{targeter, ScriptItemKind, TargeterData};
use crate::{
    area_feedback_text::Params, center_i32, dist, is_within, AreaState, EntityState, GameState,
    RangeIndicator, Script, TurnManager,
};

#[derive(Clone)]
pub enum Shape {
    Single,
    Circle {
        min_radius: f32,
        radius: f32,
    },
    Line {
        size: String,
        origin_x: i32,
        origin_y: i32,
        length: i32,
    },
    LineSegment {
        size: String,
        origin_x: i32,
        origin_y: i32,
    },
    ObjectSize {
        size: String,
    },
    Cone {
        origin_x: f32,
        origin_y: f32,
        min_radius: f32,
        radius: f32,
        angle: f32,
    },
}

fn contains(target: &Rc<RefCell<EntityState>>, list: &[Rc<RefCell<EntityState>>]) -> bool {
    for entity in list.iter() {
        if Rc::ptr_eq(target, entity) {
            return true;
        }
    }

    false
}

fn cast_high(start: Point, end: Point) -> Vec<Point> {
    let mut points = Vec::new();

    let mut delta_x = end.x - start.x;
    let delta_y = end.y - start.y;
    let xi = if delta_x < 0 {
        delta_x = -delta_x;
        -1
    } else {
        1
    };

    let mut d = 2 * delta_x - delta_y;
    let mut x = start.x;
    for y in start.y..end.y {
        points.push(Point::new(x, y));

        if d > 0 {
            x += xi;
            d -= 2 * delta_y;
        }
        d += 2 * delta_x;
    }

    points.push(Point::new(end.x, end.y));
    points
}

fn cast_low(start: Point, end: Point) -> Vec<Point> {
    let mut points = Vec::new();

    let mut delta_y = end.y - start.y;
    let delta_x = end.x - start.x;
    let yi = if delta_y < 0 {
        delta_y = -delta_y;
        -1
    } else {
        1
    };

    let mut d = 2 * delta_y - delta_x;
    let mut y = start.y;
    for x in start.x..end.x {
        points.push(Point::new(x, y));

        if d > 0 {
            y += yi;
            d -= 2 * delta_x;
        }
        d += 2 * delta_y;
    }

    points.push(Point::new(end.x, end.y));
    points
}

fn get_cursor_offset_from_size(size: &str) -> Point {
    let size = match Module::object_size(size) {
        None => {
            warn!("Invalid object size in Targeter: '{}'", size);
            return Point::default();
        }
        Some(size) => size,
    };

    Point::new(size.width / 2, size.height / 2)
}

#[derive(Copy, Clone, Debug)]
struct LosParams {
    impass_blocks: bool,
    invis_blocks: bool,
    src_elev: u8,
}

impl Shape {
    pub fn get_cursor_offset(&self) -> Point {
        use Shape::*;
        match self {
            Single | Circle { .. } | Cone { .. } => Point::default(),
            LineSegment { ref size, .. } | Line { ref size, .. } | ObjectSize { ref size } => {
                get_cursor_offset_from_size(size)
            }
        }
    }

    pub fn get_points(
        &self,
        pos: Point,
        shift: f32,
        allow_impass: bool,
        allow_invis: bool,
        impass_blocks: bool,
        invis_blocks: bool,
    ) -> Vec<Point> {
        let area_state = GameState::area_state();
        let area_state = area_state.borrow();

        let (origin_x, origin_y) = match &self {
            Shape::Single | Shape::Circle { .. } => (pos.x as f32, pos.y as f32),
            Shape::Cone {
                origin_x, origin_y, ..
            } => (*origin_x, *origin_y),
            Shape::Line {
                origin_x, origin_y, ..
            } => (*origin_x as f32, *origin_y as f32),
            Shape::LineSegment {
                origin_x, origin_y, ..
            } => (*origin_x as f32, *origin_y as f32),
            Shape::ObjectSize { ref size } => {
                let offset = get_cursor_offset_from_size(size);
                (
                    pos.x as f32 + offset.x as f32,
                    pos.y as f32 + offset.y as f32,
                )
            }
        };
        let src_elev = area_state
            .area
            .layer_set
            .elevation(origin_x as i32, origin_y as i32);

        let mut points = match self {
            Shape::Single => Vec::new(),
            Shape::Circle { min_radius, radius } => {
                self.get_points_circle(*min_radius, *radius, pos, shift, &area_state)
            }
            Shape::Line {
                ref size,
                origin_x,
                origin_y,
                length,
            } => self.get_points_line(
                Point::new(*origin_x, *origin_y),
                pos,
                *length,
                size,
                &area_state,
                LosParams {
                    impass_blocks,
                    invis_blocks,
                    src_elev,
                },
            ),
            Shape::LineSegment {
                ref size,
                origin_x,
                origin_y,
            } => self.get_points_line_segment(
                Point::new(*origin_x, *origin_y),
                pos,
                size,
                &area_state,
                LosParams {
                    impass_blocks,
                    invis_blocks,
                    src_elev,
                },
            ),
            Shape::ObjectSize { ref size } => self.get_points_object_size(pos, size, &area_state),
            Shape::Cone {
                origin_x,
                origin_y,
                min_radius,
                radius,
                angle,
            } => self.get_points_cone(*origin_x, *origin_y, pos, *min_radius, *radius, *angle),
        };

        if !allow_impass {
            points.retain(|p| {
                if !area_state.area.area.coords_valid(p.x, p.y) {
                    return false;
                }

                let index = (p.x + p.y * area_state.area.width) as usize;
                if !area_state.props().pass_grid(index) {
                    return false;
                }

                area_state.area.layer_set.is_passable_index(index)
            });
        }

        if !allow_invis {
            points.retain(|p| {
                if !area_state.area.area.coords_valid(p.x, p.y) {
                    return false;
                }

                let index = (p.x + p.y * area_state.area.width) as usize;
                if !area_state.props().vis_grid(index) {
                    return false;
                }

                if area_state.area.layer_set.elevation_index(index) > src_elev {
                    return false;
                }

                area_state.area.layer_set.is_visible_index(index)
            });
        }

        let los_params = LosParams {
            impass_blocks,
            invis_blocks,
            src_elev,
        };

        if impass_blocks || invis_blocks {
            let start = Point::new(origin_x as i32, origin_y as i32);
            let size = "1by1"; // TODO don't hardcode this
            match &self {
                Shape::ObjectSize { .. } | Shape::Cone { .. } | Shape::Circle { .. } => {
                    points.retain(|p| {
                        let (_, concat) =
                            self.get_points_line_internal(start, *p, size, &area_state, los_params);
                        !concat
                    });
                }
                Shape::Single | Shape::Line { .. } | Shape::LineSegment { .. } => (),
            }
        }

        points
    }

    pub fn get_effected_entities(
        &self,
        points: &[Point],
        target: Option<&Rc<RefCell<EntityState>>>,
        effectable: &[Rc<RefCell<EntityState>>],
    ) -> Vec<Rc<RefCell<EntityState>>> {
        match self {
            Shape::Single => match target {
                None => Vec::new(),
                Some(ref target) => {
                    if contains(target, effectable) {
                        vec![Rc::clone(target)]
                    } else {
                        Vec::new()
                    }
                }
            },
            _ => self.get_effected(points, effectable),
        }
    }

    fn get_effected(
        &self,
        points: &[Point],
        effectable: &[Rc<RefCell<EntityState>>],
    ) -> Vec<Rc<RefCell<EntityState>>> {
        let mut effected = Vec::new();

        let area_state = GameState::area_state();
        let area_state = area_state.borrow();
        for p in points.iter() {
            let entity = match area_state.get_entity_at(p.x, p.y) {
                None => continue,
                Some(entity) => entity,
            };

            if !contains(&entity, effectable) {
                continue;
            }

            if contains(&entity, &effected) {
                continue;
            }

            effected.push(entity);
        }

        effected
    }

    fn get_points_line_segment(
        &self,
        start: Point,
        end: Point,
        size: &str,
        area_state: &AreaState,
        los_params: LosParams,
    ) -> Vec<Point> {
        trace!(
            "Computing line seg points from {},{} to {},{}",
            start.x,
            start.y,
            end.x,
            end.y
        );

        let (points, concat) =
            self.get_points_line_internal(start, end, size, area_state, los_params);

        if concat {
            return Vec::new();
        }

        points
    }

    fn get_points_line(
        &self,
        start: Point,
        pos: Point,
        len: i32,
        size: &str,
        area_state: &AreaState,
        los_params: LosParams,
    ) -> Vec<Point> {
        if start.x == pos.x && start.y == pos.y {
            return Vec::new();
        }

        trace!(
            "Compute line end from start {},{}, len {}, pos {},{}",
            start.x,
            start.y,
            len,
            pos.x,
            pos.y
        );
        let dir_x = pos.x - start.x;
        let dir_y = pos.y - start.y;

        let dir_len = (dir_x * dir_x) + (dir_y * dir_y);
        let dir_len_sqrt = (dir_len as f32).sqrt();

        let dir_x = dir_x as f32 / dir_len_sqrt;
        let dir_y = dir_y as f32 / dir_len_sqrt;

        let end_x = (start.x as f32 + dir_x * len as f32).round();
        let end_y = (start.y as f32 + dir_y * len as f32).round();

        assert!(end_x.is_finite());
        assert!(end_y.is_finite());

        trace!(
            "Computing line points from {},{} to {},{}",
            start.x,
            start.y,
            end_x,
            end_y
        );

        let (points, _) = self.get_points_line_internal(
            start,
            Point::new(end_x as i32, end_y as i32),
            size,
            area_state,
            los_params,
        );

        points
    }

    #[allow(clippy::collapsible_if)]
    fn get_points_line_internal(
        &self,
        start: Point,
        end: Point,
        size: &str,
        area_state: &AreaState,
        los_params: LosParams,
    ) -> (Vec<Point>, bool) {
        let size = match Module::object_size(size) {
            None => {
                warn!("Invalid object size in Targeter: '{}'", size);
                return (Vec::new(), true);
            }
            Some(size) => size,
        };

        let (points, concat) = if (end.y - start.y).abs() < (end.x - start.x).abs() {
            if start.x > end.x {
                let mut p = cast_low(end, start);
                let concated = self.concat_from_end(&area_state, &size, &mut p, los_params);
                (p, concated)
            } else {
                let mut p = cast_low(start, end);
                let concated = self.concat_from_start(&area_state, &size, &mut p, los_params);
                (p, concated)
            }
        } else {
            if start.y > end.y {
                let mut p = cast_high(end, start);
                let concated = self.concat_from_end(&area_state, &size, &mut p, los_params);
                (p, concated)
            } else {
                let mut p = cast_high(start, end);
                let concated = self.concat_from_start(&area_state, &size, &mut p, los_params);
                (p, concated)
            }
        };

        let mut result = Vec::new();
        for p in points {
            size.points(p.x, p.y).for_each(|p| result.push(p));
        }

        result.sort();
        result.dedup();
        (result, concat)
    }

    fn check_concat_break(
        &self,
        area: &AreaState,
        size: &ObjectSize,
        p: Point,
        los_params: LosParams,
    ) -> bool {
        let p_index = (p.x + p.y * area.area.width) as usize;

        if los_params.impass_blocks {
            if !area.is_terrain_passable(&size.id, p.x, p.y) {
                return true;
            }

            if !area.props().pass_grid(p_index) {
                return true;
            }
        }

        if los_params.invis_blocks {
            if !area.props().vis_grid(p_index) {
                return true;
            }

            if area.area.layer_set.elevation_index(p_index) > los_params.src_elev {
                return true;
            }

            if !area.area.layer_set.is_visible_index(p_index) {
                return true;
            }
        }

        false
    }

    fn concat_from_start(
        &self,
        area: &AreaState,
        size: &ObjectSize,
        points: &mut Vec<Point>,
        los_params: LosParams,
    ) -> bool {
        let mut index = 0;
        loop {
            if index == points.len() {
                return false;
            }

            if self.check_concat_break(area, size, points[index], los_params) {
                break;
            }

            index += 1;
        }

        if index == 0 {
            points.clear();
        } else {
            points.truncate(index);
        }

        true
    }

    fn concat_from_end(
        &self,
        area: &AreaState,
        size: &ObjectSize,
        points: &mut Vec<Point>,
        los_params: LosParams,
    ) -> bool {
        let mut index = points.len() - 1;
        loop {
            if self.check_concat_break(area, size, points[index], los_params) {
                break;
            }

            if index == 0 {
                return false;
            }
            index -= 1;
        }

        if index == 0 {
            points.remove(0);
        } else {
            points.drain(0..=index);
        }

        true
    }

    fn get_points_cone(
        &self,
        origin_x: f32,
        origin_y: f32,
        to: Point,
        min_radius: f32,
        radius: f32,
        angular_size: f32,
    ) -> Vec<Point> {
        let pos = Point::new(origin_x.trunc() as i32, origin_y.trunc() as i32);
        let origin = (origin_x, origin_y);
        let angle = (to.y as f32 - origin_y).atan2(to.x as f32 - origin_x);

        let mut points = Vec::new();

        let r = (radius + 2.0).ceil() as i32;
        let width = (r * 2) as usize;

        for y in 0..width {
            for x in 0..width {
                let x1 = x as i32 + pos.x - r;
                let y1 = y as i32 + pos.y - r;
                let p = Point::new(x1, y1);

                let dist = dist(&origin, &p);
                if dist > radius || dist < min_radius {
                    continue;
                }

                let cur_angle = (y as f32 - r as f32).atan2(x as f32 - r as f32);
                let angle_diff = (angle - cur_angle + 3.0 * PI) % (2.0 * PI) - PI;
                if angle_diff.abs() > angular_size / 2.0 {
                    continue;
                }

                points.push(p);
            }
        }

        points
    }

    fn get_points_circle(
        &self,
        min_radius: f32,
        radius: f32,
        pos: Point,
        shift: f32,
        _area_state: &AreaState,
    ) -> Vec<Point> {
        let origin = (pos.x as f32 - shift, pos.y as f32 - shift);
        let mut points = Vec::new();

        let r = (radius + 2.0).ceil() as i32;
        let width = (r * 2) as usize;

        for y in 0..width {
            for x in 0..width {
                let x1 = x as i32 + pos.x - r;
                let y1 = y as i32 + pos.y - r;
                let p = Point::new(x1, y1);

                let dist = dist(&origin, &p);

                if dist <= radius && dist >= min_radius {
                    points.push(p);
                }
            }
        }

        points
    }

    fn get_points_object_size(
        &self,
        pos: Point,
        size: &str,
        _area_state: &AreaState,
    ) -> Vec<Point> {
        let size = match Module::object_size(size) {
            None => {
                warn!("Invalid object size in Targeter: '{}'", size);
                return Vec::new();
            }
            Some(size) => size,
        };
        size.points(pos.x, pos.y).collect()
    }
}

enum ScriptSource {
    Ability(Rc<Ability>),
    Item { kind: ScriptItemKind, name: String },
}

/// A created AreaTargeter, built from a `Targeter`

pub struct AreaTargeter {
    on_target_select_func: String,
    on_target_select_custom_target: Option<Rc<RefCell<EntityState>>>,
    script_source: ScriptSource,
    parent: Rc<RefCell<EntityState>>,
    selectable: Vec<Rc<RefCell<EntityState>>>,
    effectable: Vec<Rc<RefCell<EntityState>>>,
    max_effectable: Option<usize>,
    shape: Shape,
    show_mouseover: bool,
    free_select: Option<f32>,
    range_indicator: Option<RangeIndicator>,
    free_select_must_be_passable: Option<Rc<ObjectSize>>,
    allow_affected_points_impass: bool,
    allow_affected_points_invis: bool,
    impass_blocks_affected_points: bool,
    invis_blocks_affected_points: bool,

    free_select_valid: bool,
    cur_target: Option<Rc<RefCell<EntityState>>>,
    cursor_pos: Point,
    cursor_offset: Point,
    cur_points: Vec<Point>,
    cur_effected: Vec<Rc<RefCell<EntityState>>>,

    cancel: bool,
}

fn create_entity_state_vec(
    mgr: &TurnManager,
    input: &[Option<usize>],
) -> Vec<Rc<RefCell<EntityState>>> {
    let mut out = Vec::new();
    for index in input.iter() {
        let index = match index {
            None => continue,
            Some(index) => *index,
        };

        match mgr.entity_checked(index) {
            None => (),
            Some(entity) => out.push(entity),
        }
    }
    out
}

impl AreaTargeter {
    pub fn from(data: &TargeterData) -> AreaTargeter {
        let mgr = GameState::turn_manager();
        let mgr = mgr.borrow();

        let free_select_must_be_passable = match data.free_select_must_be_passable {
            None => None,
            Some(ref size) => match Module::object_size(size) {
                None => {
                    warn!("Invalid object size in Targeter: '{}'", size);
                    None
                }
                Some(size) => Some(size),
            },
        };

        let parent = mgr.entity(data.parent);

        let script_source = match &data.kind {
            targeter::Kind::Ability(ref id) => ScriptSource::Ability(Module::ability(id).unwrap()),
            targeter::Kind::Item(kind) => {
                let name = match kind.item_checked(&parent) {
                    None => {
                        warn!("Invalid item kind for targeter");
                        "".to_string()
                    }
                    Some(item_state) => item_state.get_name(),
                };
                ScriptSource::Item {
                    kind: kind.clone(),
                    name,
                }
            }
        };

        let range_indicator = match data.selection_area {
            targeter::SelectionArea::None => None,
            targeter::SelectionArea::Radius(radius) => {
                Some(RangeIndicator::targeter(radius, &parent))
            }
            targeter::SelectionArea::Visible => {
                let area = GameState::area_state();
                let r = area.borrow().area.area.vis_dist;
                Some(RangeIndicator::targeter(r as f32, &parent))
            }
            targeter::SelectionArea::Attackable => {
                let r = parent.borrow().actor.stats.attack_distance();
                Some(RangeIndicator::targeter(r, &parent))
            }
            targeter::SelectionArea::Touchable => {
                let r = parent.borrow().actor.stats.touch_distance();
                Some(RangeIndicator::targeter(r, &parent))
            }
        };

        AreaTargeter {
            on_target_select_func: data.on_target_select_func.to_string(),
            on_target_select_custom_target: match data.on_target_select_custom_target {
                None => None,
                Some(index) => mgr.entity_checked(index),
            },
            script_source,
            parent,
            selectable: create_entity_state_vec(&mgr, &data.selectable),
            effectable: create_entity_state_vec(&mgr, &data.effectable),
            max_effectable: data.max_effectable,
            cancel: false,
            free_select: data.free_select,
            range_indicator,
            free_select_must_be_passable,
            allow_affected_points_impass: data.allow_affected_points_impass,
            allow_affected_points_invis: data.allow_affected_points_invis,
            impass_blocks_affected_points: data.impass_blocks_affected_points,
            invis_blocks_affected_points: data.invis_blocks_affected_points,
            free_select_valid: false,
            show_mouseover: data.show_mouseover,
            cur_target: None,
            cursor_pos: Point::default(),
            cursor_offset: Point::default(),
            cur_points: Vec::new(),
            cur_effected: Vec::new(),
            shape: data.shape.clone(),
        }
    }

    pub fn take_range_indicator(&mut self) -> Option<RangeIndicator> {
        self.range_indicator.take()
    }

    fn draw_target(&self, target: &Rc<RefCell<EntityState>>, offset: Offset) -> DrawList {
        let target = target.borrow();
        let rect = Rect {
            x: target.location.x as f32 - offset.x,
            y: target.location.y as f32 - offset.y,
            w: target.size.width as f32,
            h: target.size.height as f32,
        };

        DrawList::from_sprite_f32(&target.size.cursor_sprite, rect)
    }

    fn calculate_points(&mut self) {
        self.cur_points.clear();
        self.cur_effected.clear();

        if self.free_select.is_none() {
            let target = match self.cur_target {
                None => return,
                Some(ref target) => target,
            };

            let (mut center_x, mut center_y) = center_i32(&*target.borrow());
            center_x -= self.cursor_offset.x;
            center_y -= self.cursor_offset.y;
            let shift = if target.borrow().size.width % 2 == 0 {
                0.5
            } else {
                0.0
            };

            self.cur_points = self.shape.get_points(
                Point::new(center_x, center_y),
                shift,
                self.allow_affected_points_impass,
                self.allow_affected_points_invis,
                self.impass_blocks_affected_points,
                self.invis_blocks_affected_points,
            );
            self.cur_effected =
                self.shape
                    .get_effected_entities(&self.cur_points, Some(&target), &self.effectable);
        } else {
            if !self.free_select_valid {
                return;
            }

            let pos = self.cursor_pos - self.cursor_offset;
            self.cur_points = self.shape.get_points(
                pos,
                0.0,
                self.allow_affected_points_impass,
                self.allow_affected_points_invis,
                self.impass_blocks_affected_points,
                self.invis_blocks_affected_points,
            );
            self.cur_effected =
                self.shape
                    .get_effected_entities(&self.cur_points, None, &self.effectable);

            if self.cur_points.is_empty() {
                self.free_select_valid = false;
            }
        }

        if let Some(max) = self.max_effectable {
            self.cur_effected.truncate(max);
        }
    }

    fn compute_free_select_valid(&mut self) -> bool {
        let max_dist = match self.free_select {
            None => {
                return false;
            }
            Some(dist) => dist,
        };

        if !is_within(&*self.parent.borrow(), &self.cursor_pos, max_dist) {
            return false;
        }

        let area_state = GameState::area_state();
        if !area_state
            .borrow()
            .is_pc_visible(self.cursor_pos.x, self.cursor_pos.y)
        {
            // TODO use the parent's visibility since it doesn't have to be a PC
            return false;
        }

        if let Some(ref size) = self.free_select_must_be_passable {
            if !area_state.borrow().is_passable_size(
                size,
                self.cursor_pos.x - size.width / 2,
                self.cursor_pos.y - size.height / 2,
            ) {
                return false;
            }
        }

        true
    }

    pub fn parent(&self) -> &Rc<RefCell<EntityState>> {
        &self.parent
    }

    pub fn name(&self) -> &str {
        match &self.script_source {
            ScriptSource::Ability(ability) => &ability.name,
            ScriptSource::Item { name, .. } => name,
        }
    }

    pub fn cancel(&self) -> bool {
        self.cancel
    }

    pub fn draw(
        &mut self,
        renderer: &mut dyn GraphicsRenderer,
        tile: &Rc<dyn Image>,
        offset: Offset,
        scale: Scale,
        millis: u32,
        params: &Params,
    ) {
        if !self.parent.borrow().is_party_member() {
            return;
        }

        let mut draw_list = DrawList::empty_sprite();

        for target in self.selectable.iter() {
            draw_list.append(&mut self.draw_target(target, offset));
        }

        if !draw_list.is_empty() {
            draw_list.set_scale(scale);
            renderer.draw(draw_list);
        }

        let mut draw_list = DrawList::empty_sprite();
        for target in self.cur_effected.iter() {
            draw_list.append(&mut self.draw_target(target, offset));
        }
        draw_list.set_scale(scale);
        draw_list.set_color(color::RED);
        renderer.draw(draw_list);

        let mut draw_list = DrawList::empty_sprite();
        for p in self.cur_points.iter() {
            let rect = Rect {
                x: p.x as f32 - offset.x,
                y: p.y as f32 - offset.y,
                w: 1.0,
                h: 1.0,
            };
            tile.append_to_draw_list(&mut draw_list, &animation_state::NORMAL, rect, millis);
        }
        draw_list.set_scale(scale);
        renderer.draw(draw_list);

        if let ScriptSource::Ability(ability) = &self.script_source {
            if let Some(active) = &ability.active {
                self.draw_ap_usage(renderer, params, offset, scale, active.ap as i32);
            }
        }
    }

    fn draw_ap_usage(
        &self,
        renderer: &mut dyn GraphicsRenderer,
        params: &Params,
        offset: Offset,
        scale: Scale,
        ap: i32,
    ) {
        if !GameState::is_combat_active() {
            return;
        }

        let parent = &self.parent.borrow();
        let ap = parent.actor.ap() as i32 - ap;

        // compute position to show AP and do nothing if not valid to activate
        let (x, y) = if self.free_select.is_none() {
            match &self.cur_target {
                None => return,
                Some(target) => {
                    let target = &target.borrow();
                    (
                        target.location.x as f32,
                        target.location.y as f32 + target.size.height as f32,
                    )
                }
            }
        } else {
            if !self.free_select_valid {
                return;
            }
            (self.cursor_pos.x as f32, self.cursor_pos.y as f32 + 1.0)
        };

        let font_rend = LineRenderer::new(&params.font);
        let text = format!("{} AP", Module::rules().format_ap(ap));
        let x = x - offset.x;
        let y = y - offset.y;
        let offset = Offset { x, y };
        let (mut draw_list, _) = font_rend.get_draw_list(&text, offset, params.ap_scale);
        draw_list.set_color(params.ap_color);
        draw_list.set_scale(scale);
        renderer.draw(draw_list);
    }

    pub fn on_mouse_move(
        &mut self,
        cursor_x: i32,
        cursor_y: i32,
    ) -> Option<&Rc<RefCell<EntityState>>> {
        self.cursor_pos = Point::new(cursor_x, cursor_y);
        self.cursor_offset = self.shape.get_cursor_offset();
        self.cur_target = None;

        for target in self.selectable.iter() {
            {
                let target = target.borrow();
                let x1 = target.location.x;
                let y1 = target.location.y;
                let x2 = x1 + target.size.width - 1;
                let y2 = y1 + target.size.height - 1;

                if cursor_x < x1 || cursor_x > x2 || cursor_y < y1 || cursor_y > y2 {
                    continue;
                }
            }

            self.cur_target = Some(Rc::clone(target));
            break;
        }

        self.free_select_valid = self.compute_free_select_valid();
        self.calculate_points();

        let kind = if self.free_select.is_none() {
            match self.cur_target {
                None => animation_state::Kind::MouseInvalid,
                Some(_) => animation_state::Kind::MouseSelect,
            }
        } else if self.free_select_valid {
            animation_state::Kind::MouseSelect
        } else {
            animation_state::Kind::MouseInvalid
        };
        Cursor::set_cursor_state(kind);

        if self.show_mouseover {
            self.cur_target.as_ref()
        } else {
            None
        }
    }

    pub fn on_cancel(&mut self) {
        self.cancel = true;
    }

    pub fn is_free_select(&self) -> bool {
        self.free_select.is_some()
    }

    pub fn selectable(&self) -> &[Rc<RefCell<EntityState>>] {
        &self.selectable
    }

    pub fn cur_affected(&self) -> &Vec<Rc<RefCell<EntityState>>> {
        &self.cur_effected
    }

    pub fn is_valid_to_activate(&self) -> bool {
        if self.cancel() {
            return false;
        }

        if self.free_select.is_none() {
            self.cur_target.is_some()
        } else {
            self.free_select_valid
        }
    }

    pub fn on_activate(&mut self) {
        if !self.is_valid_to_activate() {
            return;
        }

        self.cancel = true;

        let affected = self
            .cur_effected
            .iter()
            .map(|e| Some(Rc::clone(e)))
            .collect();

        let mut pos = self.cursor_pos;
        if let Some(ref size) = self.free_select_must_be_passable {
            pos.x -= size.width / 2;
            pos.y -= size.height / 2;
        }

        let points = self.cur_points.clone();
        let func = &self.on_target_select_func;
        let custom_target = self.on_target_select_custom_target.clone();
        info!("on target select script");
        match &self.script_source {
            ScriptSource::Ability(ref ability) => Script::ability_on_target_select(
                &self.parent,
                ability,
                affected,
                pos,
                points,
                func,
                custom_target,
            ),
            ScriptSource::Item { kind, .. } => Script::item_on_target_select(
                &self.parent,
                kind.clone(),
                affected,
                pos,
                points,
                func,
                custom_target,
            ),
        }
    }
}
