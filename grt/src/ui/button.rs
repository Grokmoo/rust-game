use std::rc::Rc;
use std::cell::RefCell;

use ui::{Callback, Label, Widget, WidgetKind};
use io::{event, GraphicsRenderer, TextRenderer};
use util::Point;

pub struct Button {
    label: Rc<Label>,
    callback: Option<Callback<Button>>,
}

impl Button {
    pub fn empty() -> Rc<Button> {
        Rc::new(Button {
            label: Label::empty(),
            callback: None
        })
    }

    pub fn new(text: &str, callback: Callback<Button>) -> Rc<Button> {
        Rc::new(Button {
            label: Label::new(text),
            callback: Some(callback),
        })
    }

    pub fn with_callback(callback: Callback<Button>) -> Rc<Button> {
        Rc::new(Button {
            label: Label::empty(),
            callback: Some(callback)
        })
    }

    pub fn with_text(text: &str) -> Rc<Button> {
        Rc::new(Button {
            label: Label::new(text),
            callback: None
        })
    }
}

impl WidgetKind for Button {
    fn get_name(&self) -> &str {
        "button"
    }

    fn layout(&self, widget: &mut Widget) {
        if let Some(ref text) = self.label.text {
            widget.state.add_text_arg(text);
        }
        widget.do_base_layout();
    }

    fn draw_text_mode(&self, renderer: &mut TextRenderer,
                      widget: &Widget, millis: u32) {
        self.label.draw_text_mode(renderer, widget, millis);
    }

    fn draw_graphics_mode(&self, renderer: &mut GraphicsRenderer, pixel_size: Point,
                          widget: &Widget, millis: u32) {
        self.label.draw_graphics_mode(renderer, pixel_size, widget, millis);
    }

    fn on_mouse_release(&self, widget: &Rc<RefCell<Widget>>, kind: event::ClickKind) -> bool {
        self.super_on_mouse_release(widget, kind);
        match self.callback {
            Some(ref cb) => cb.call(self, widget),
            None => (),
        };
        true
    }
}
