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

use std::fmt;
use std::io::Error;
use std::rc::Rc;

use sulis_core::io::{DrawList, GraphicsRenderer};
use sulis_core::ui::{animation_state, AnimationState, Color};
use sulis_core::util::{self, invalid_data_error, Offset, Scale};
use sulis_module::area::PropData;
use sulis_module::{prop, ItemState, LootList, Module, ObjectSizeIterator, Prop};

use crate::entity_state::AreaDrawable;
use crate::save_state::PropInteractiveSaveState;
use crate::{ChangeListenerList, EntityTextureCache, ItemList, Location};

#[derive(Debug)]
pub enum Interactive {
    Not,
    Container {
        items: ItemList,
        loot_to_generate: Option<Rc<LootList>>,
        temporary: bool,
    },
    Door {
        open: bool,
    },
    Hover {
        text: String,
    },
}

pub struct PropState {
    pub prop: Rc<Prop>,
    pub location: Location,
    pub animation_state: AnimationState,
    pub listeners: ChangeListenerList<PropState>,
    pub(crate) interactive: Interactive,
    enabled: bool,

    marked_for_removal: bool,

    millis_offset: u32,
}

impl fmt::Debug for PropState {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Prop: {:?}", self.prop)?;
        write!(f, "Location: {:?}", self.location)
    }
}

impl PropState {
    pub(crate) fn new(prop_data: &PropData, location: Location, temporary: bool) -> PropState {
        let mut items = ItemList::default();
        for item_save in prop_data.items.iter() {
            let quantity = item_save.quantity;
            let item = &item_save.item;
            let variant = item.variant;
            let identified = item.identified;
            let item = match Module::create_get_item(&item.id, &item.adjectives) {
                None => {
                    warn!(
                        "Unable to create item '{}' with '{:?}' in prop '{}'",
                        item_save.item.id, item_save.item.adjectives, prop_data.prop.id
                    );
                    continue;
                }
                Some(item) => item,
            };
            items.add_quantity(quantity, ItemState::new(item, variant, identified));
        }

        let mut anim_state = AnimationState::default();

        let interactive = match prop_data.prop.interactive {
            prop::Interactive::Hover => {
                let text = prop_data.hover_text.clone().unwrap_or_default();
                Interactive::Hover { text }
            }
            prop::Interactive::Not => {
                if !items.is_empty() {
                    warn!("Attempted to add items to a non-container prop");
                }
                Interactive::Not
            }
            prop::Interactive::Container { ref loot } => Interactive::Container {
                items,
                loot_to_generate: loot.clone(),
                temporary,
            },
            prop::Interactive::Door { initially_open, .. } => {
                if initially_open {
                    anim_state.toggle(animation_state::Kind::Active);
                }

                Interactive::Door {
                    open: initially_open,
                }
            }
        };

        let millis_offset_range = prop_data.prop.random_millis_offset;
        let millis_offset = if millis_offset_range == 0 {
            0
        } else {
            util::gen_rand(0, millis_offset_range)
        };

        PropState {
            prop: Rc::clone(&prop_data.prop),
            enabled: prop_data.enabled,
            location,
            interactive,
            animation_state: anim_state,
            listeners: ChangeListenerList::default(),
            marked_for_removal: false,
            millis_offset,
        }
    }

    pub(crate) fn load_interactive(
        &mut self,
        interactive: PropInteractiveSaveState,
    ) -> Result<(), Error> {
        match interactive {
            PropInteractiveSaveState::Not => {
                // the base prop interactive must match, if not don't load this.
                // this is for save compat.
                match self.prop.interactive {
                    prop::Interactive::Not => (),
                    _ => return Ok(()),
                }
                self.interactive = Interactive::Not;
            }
            PropInteractiveSaveState::Container {
                loot_to_generate,
                temporary,
                items,
            } => {
                // the base prop interactive must match, if not don't load this.
                // this is for save compat.
                match self.prop.interactive {
                    prop::Interactive::Container { .. } => (),
                    _ => return Ok(()),
                }

                let mut item_list = ItemList::default();
                for item_save_state in items {
                    let item = &item_save_state.item;
                    let variant = item.variant;
                    let identified = !item.adjectives.contains(&Module::rules().unidentified_item_adjective);
                    let item = match Module::create_get_item(&item.id, &item.adjectives) {
                        None => invalid_data_error(&format!(
                            "No item with ID '{}'",
                            item_save_state.item.id
                        )),
                        Some(item) => Ok(item),
                    }?;

                    item_list.add_quantity(item_save_state.quantity, ItemState::new(item, variant, identified));
                }

                let loot = match loot_to_generate {
                    None => Ok(None),
                    Some(ref id) => match Module::loot_list(id) {
                        None => invalid_data_error(&format!("No loot list with ID '{}'", id)),
                        Some(loot_list) => Ok(Some(loot_list)),
                    },
                }?;

                self.interactive = Interactive::Container {
                    items: item_list,
                    loot_to_generate: loot,
                    temporary,
                };
            }
            PropInteractiveSaveState::Door { open } => {
                // the base prop interactive must match, if not don't load this.
                // this is for save compat.
                match self.prop.interactive {
                    prop::Interactive::Door { .. } => (),
                    _ => return Ok(()),
                }

                self.interactive = Interactive::Door { open };

                if open {
                    self.animation_state.add(animation_state::Kind::Active);
                } else {
                    self.animation_state.remove(animation_state::Kind::Active);
                }
            }
            PropInteractiveSaveState::Hover { text } => {
                // the base prop interactive must match, if not don't load this.
                // this is for save compat.
                match self.prop.interactive {
                    prop::Interactive::Hover { .. } => (),
                    _ => return Ok(()),
                }

                self.interactive = Interactive::Hover { text };
            }
        }

        Ok(())
    }

    pub fn name(&self) -> &str {
        match self.interactive {
            Interactive::Hover { ref text } => text,
            _ => &self.prop.name,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn location_points(&self) -> ObjectSizeIterator {
        self.prop.size.points(self.location.x, self.location.y)
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub(crate) fn is_marked_for_removal(&self) -> bool {
        self.marked_for_removal
    }

    pub fn might_contain_items(&self) -> bool {
        match self.interactive {
            Interactive::Container {
                ref items,
                ref loot_to_generate,
                ..
            } => {
                if !items.is_empty() {
                    return true;
                }
                if loot_to_generate.is_some() {
                    return true;
                }

                false
            }
            _ => false,
        }
    }

    pub fn is_door(&self) -> bool {
        match self.interactive {
            Interactive::Door { .. } => true,
            _ => false,
        }
    }

    pub fn is_hover(&self) -> bool {
        match self.interactive {
            Interactive::Hover { .. } => true,
            _ => false,
        }
    }

    pub fn is_container(&self) -> bool {
        match self.interactive {
            Interactive::Container { .. } => true,
            _ => false,
        }
    }

    pub fn toggle_active(&mut self) {
        self.animation_state.toggle(animation_state::Kind::Active);
        let is_active = self.is_active();

        match self.interactive {
            Interactive::Not | Interactive::Hover { .. } => (),
            Interactive::Container {
                ref mut items,
                ref mut loot_to_generate,
                ..
            } => {
                if !is_active {
                    return;
                }

                let loot = match loot_to_generate.take() {
                    None => return,
                    Some(loot) => loot,
                };

                info!("Generating loot for prop from '{}'", loot.id);
                let generated_items = loot.generate();
                for (qty, item) in generated_items {
                    items.add_quantity(qty, item);
                }
            }
            Interactive::Door { ref mut open } => {
                let cur_open = *open;
                *open = !cur_open;
            }
        }
    }

    pub fn add_item(&mut self, item: ItemState) {
        match self.interactive {
            Interactive::Container { ref mut items, .. } => {
                items.add(item);
            }
            _ => warn!(
                "Attempted to add item to a non-container prop {}",
                self.prop.id
            ),
        }
        self.listeners.notify(&self);
    }

    pub fn add_items(&mut self, items_to_add: Vec<(u32, ItemState)>) {
        match self.interactive {
            Interactive::Container { ref mut items, .. } => {
                for (qty, item) in items_to_add {
                    items.add_quantity(qty, item);
                }
            }
            _ => warn!(
                "Attempted to add items to a non-container prop {}",
                self.prop.id
            ),
        }
        self.listeners.notify(&self);
    }

    pub fn items(&self) -> Option<&ItemList> {
        match self.interactive {
            Interactive::Container { ref items, .. } => Some(&items),
            _ => None,
        }
    }

    pub fn remove_all_at(&mut self, index: usize) -> Option<(u32, ItemState)> {
        let item_state = match self.interactive {
            Interactive::Container { ref mut items, .. } => items.remove_all_at(index),
            _ => {
                warn!(
                    "Attempted to remove items from a non-container prop {}",
                    self.prop.id
                );
                None
            }
        };
        self.notify_and_check();
        item_state
    }

    pub fn remove_one_at(&mut self, index: usize) -> Option<ItemState> {
        let item_state = match self.interactive {
            Interactive::Container { ref mut items, .. } => items.remove(index),
            _ => {
                warn!(
                    "Attempted to remove item from a non-container prop {}",
                    self.prop.id
                );
                None
            }
        };
        self.notify_and_check();
        item_state
    }

    fn notify_and_check(&mut self) {
        self.listeners.notify(&self);
        if let Interactive::Container {
            ref items,
            temporary,
            ..
        } = self.interactive
        {
            if items.is_empty() && temporary {
                self.marked_for_removal = true;
            }
        }
    }

    pub fn is_active(&self) -> bool {
        self.animation_state.contains(animation_state::Kind::Active)
    }

    pub fn append_to_draw_list(&self, draw_list: &mut DrawList, offset: Offset, millis: u32) {
        self.prop
            .append_to_draw_list(draw_list, &self.animation_state, offset, millis);
    }
}

impl AreaDrawable for PropState {
    fn cache(
        &mut self,
        _renderer: &mut dyn GraphicsRenderer,
        _texture_cache: &mut EntityTextureCache,
    ) {
    }

    fn draw(
        &self,
        renderer: &mut dyn GraphicsRenderer,
        scale: Scale,
        x: f32,
        y: f32,
        millis: u32,
        color: Color,
    ) {
        let pos = Offset {
            x: x + self.location.x as f32,
            y: y + self.location.y as f32,
        };

        let mut draw_list = DrawList::empty_sprite();
        draw_list.set_scale(scale);
        draw_list.set_color(color);
        self.append_to_draw_list(&mut draw_list, pos, millis + self.millis_offset);
        renderer.draw(draw_list);
    }

    fn location(&self) -> &Location {
        &self.location
    }

    fn aerial(&self) -> bool {
        self.prop.aerial
    }
}
