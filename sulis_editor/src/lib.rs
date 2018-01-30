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

mod area_editor;
use area_editor::AreaEditor;

mod tile_picker;
use tile_picker::TilePicker;

#[macro_use] extern crate log;

extern crate sulis_core;
extern crate sulis_module;
extern crate sulis_widgets;

use std::rc::Rc;
use std::cell::RefCell;

use sulis_core::io::{InputAction, MainLoopUpdater};
use sulis_core::ui::{Callback, Widget, WidgetKind};
use sulis_widgets::{ConfirmationWindow, DropDown, list_box};

thread_local! {
    static EXIT: RefCell<bool> = RefCell::new(false);
}

pub struct EditorMainLoopUpdater { }

impl MainLoopUpdater for EditorMainLoopUpdater {
    fn update(&self) {

    }

    fn is_exit(&self) -> bool {
        EXIT.with(|exit| *exit.borrow())
    }
}

const NAME: &str = "editor";

pub struct EditorView {
}

impl EditorView {
    pub fn new() -> Rc<RefCell<EditorView>> {
        Rc::new(RefCell::new(EditorView { }))
    }
}

impl WidgetKind for EditorView {
    fn get_name(&self) -> &str {
        NAME
    }

    fn on_key_press(&mut self, widget: &Rc<RefCell<Widget>>, key: InputAction) -> bool {
        use InputAction::*;
        match key {
            Exit => {
                let exit_window = Widget::with_theme(ConfirmationWindow::new(Callback::with(
                            Box::new(|| { EXIT.with(|exit| *exit.borrow_mut() = true); }))),
                            "exit_confirmation_window");
                exit_window.borrow_mut().state.set_modal(true);
                Widget::add_child_to(&widget, exit_window);

            },
            _ => return false,
        }

        true
    }

    fn on_add(&mut self, _widget: &Rc<RefCell<Widget>>) -> Vec<Rc<RefCell<Widget>>> {
        debug!("Adding to editor widget");

        let tile_picker = Widget::with_defaults(TilePicker::new());

        let area_editor_kind = AreaEditor::new(&tile_picker);

        let top_bar = Widget::empty("top_bar");
        {
            let mut entries: Vec<list_box::Entry<String>> = Vec::new();

            let area_editor_kind_ref = Rc::clone(&area_editor_kind);
            let save = list_box::Entry::new("Save".to_string(), Some(Callback::new(Rc::new(move |widget| {
                let parent = Widget::get_parent(widget);
                area_editor_kind_ref.borrow().save("test.yml");
                parent.borrow_mut().mark_for_removal();
            }))));
            entries.push(save);

            let drop_down = DropDown::new(entries);
            let menu = Widget::with_theme(drop_down, "menu");

            Widget::add_child_to(&top_bar, menu);
        }

        let area_editor = Widget::with_defaults(area_editor_kind);

        vec![tile_picker, area_editor, top_bar]
    }
}