//! The page's input method, which is a hidden text element parked under the caret.
//!
//! Everything else about the overlay is winit's to report, but not this: winit's web backend has
//! `set_ime_allowed` as an empty function and never sends [`winit::event::WindowEvent::Ime`], so
//! on a page there is no input method at all unless one is built out of the browser's own. A
//! browser only reports a word being built up -- `compositionstart`, `compositionupdate`,
//! `compositionend` -- against an element that can be typed into, and a `<canvas>` is not one. So
//! an `<input>` is put on the page, made invisible, moved under wherever egui says the caret is,
//! and given the focus for as long as something is being typed.
//!
//! While the element holds the focus the canvas does not, and a canvas without the focus is sent
//! no keys -- so the keys are read off the element here too and handed on as egui's own.
//!
//! # Where this comes from
//!
//! Carried from `eframe`, which is the one piece of it that cannot be depended on instead: the
//! rest of what `eframe` does for an input method is `egui-winit`'s, and [`super::gui`] uses that
//! directly. `eframe` itself is not a dependency because it wants to own the window, the context
//! and the event loop, all three of which belong to a 3D renderer here.
//!
//! - Upstream: `crates/eframe/src/web/text_agent.rs` at tag `0.34.3`, plus `on_keydown` and
//!   `on_keyup` from `crates/eframe/src/web/events.rs` and the focus half of
//!   `AppRunner::handle_platform_output` in `crates/eframe/src/web/app_runner.rs`.
//!   <https://github.com/emilk/egui/blob/0.34.3/crates/eframe/src/web/text_agent.rs>
//! - Under MIT OR Apache-2.0, (c) Emil Ernerfeldt and the egui contributors.
//!
//! To take a later version, read what moved with
//!
//! ```notrust
//! git diff 0.34.3..<tag> -- crates/eframe/src/web/text_agent.rs crates/eframe/src/web/events.rs
//! ```
//!
//! and expect the differences below, which are this app's rather than upstream's drift:
//!
//! - Events are left on [`egui_winit::State::egui_input_mut`] for the next frame to take, where
//!   upstream pushes them onto its own `AppRunner`. That is the whole of the seam.
//! - The canvas is winit's, asked for through `WindowExtWebSys`, not one `eframe` made. So there
//!   is no `root: Node` to append to and the element goes on `<body>`.
//! - `move_to` and the focus that upstream keeps in `handle_platform_output` are one call here,
//!   [`TextAgent::follow`], because there is one caller and one place it can be made from.
//! - The keys are translated inline. Upstream's `should_prevent_default_for_key` and its
//!   `should_stop_propagation` option are dropped: this page has nothing else on it to defend a
//!   key from, so only `Tab` is held back, and only because it would take the focus away.
//! - Upstream's `is_mobile_safari` correction to the caret position is not carried, nor is
//!   `set_autofocus`. Both are worth a second look if the element ever lands in the wrong place.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::{JsCast, JsValue, prelude::Closure};

/// What the listeners collect between frames, for [`TextAgent::lend_focus`] to hand over.
#[derive(Default)]
struct Pending {
    events: RefCell<Vec<egui::Event>>,
    modifiers: Cell<egui::Modifiers>,
}

pub(crate) struct TextAgent {
    input: web_sys::HtmlInputElement,
    canvas: web_sys::HtmlCanvasElement,
    pending: Rc<Pending>,
    /// Where the caret was last put, so the element is only moved when it has actually moved.
    placed: Cell<Option<egui::output::IMEOutput>>,
    /// Kept only to be kept alive: a listener stops working the moment its closure is dropped.
    _listeners: Vec<Closure<dyn FnMut(web_sys::Event)>>,
}

impl TextAgent {
    /// Puts the element on the page and starts listening on it.
    pub(crate) fn attach(window: &winit::window::Window) -> Result<Self, JsValue> {
        use winit::platform::web::WindowExtWebSys as _;

        let canvas = window
            .canvas()
            .expect("the page has no canvas to type over");
        let document = web_sys::window().unwrap().document().unwrap();

        let input = document
            .create_element("input")?
            .dyn_into::<web_sys::HtmlInputElement>()?;
        input.set_type("text");
        // A phone would otherwise capitalise the first letter of a world's name for the person.
        input.set_attribute("autocapitalize", "off")?;
        input.set_attribute("aria-hidden", "true")?;

        // Invisible rather than hidden or off-screen: an element the page will not draw is also
        // an element the browser will not let an input method open against.
        let style = input.style();
        style.set_property("position", "absolute")?;
        style.set_property("top", "0")?;
        style.set_property("left", "0")?;
        style.set_property("width", "1px")?;
        style.set_property("height", "1px")?;
        style.set_property("border", "none")?;
        style.set_property("outline", "none")?;
        style.set_property("background-color", "transparent")?;
        style.set_property("caret-color", "transparent")?;
        document.body().unwrap().append_child(&input)?;

        let pending = Rc::<Pending>::default();
        let mut listeners = Vec::new();

        // Committed text that was never composed: an ordinary keystroke, or a paste. A composed
        // word raises this too, and is left to `compositionend` instead -- taking both would type
        // it twice.
        listen(&input, "input", &mut listeners, {
            let input = input.clone();
            let pending = Rc::clone(&pending);
            move |event: web_sys::InputEvent| {
                let text = input.value();
                if event.is_composing() {
                    return;
                }
                // Clears the suggestion strip a phone keyboard leaves behind. `eframe` does the
                // same, for the same reason.
                input.blur().ok();
                input.focus().ok();
                if !text.is_empty() {
                    input.set_value("");
                    pending.events.borrow_mut().push(egui::Event::Text(text));
                }
            }
        })?;

        listen(&input, "compositionstart", &mut listeners, {
            let input = input.clone();
            let pending = Rc::clone(&pending);
            move |_: web_sys::CompositionEvent| {
                input.set_value("");
                pending
                    .events
                    .borrow_mut()
                    .push(egui::Event::Ime(egui::ImeEvent::Enabled));
            }
        })?;

        // The word as it stands so far, which egui draws underlined in the field itself rather
        // than leaving it to the candidate window alone.
        listen(&input, "compositionupdate", &mut listeners, {
            let pending = Rc::clone(&pending);
            move |event: web_sys::CompositionEvent| {
                if let Some(text) = event.data() {
                    pending
                        .events
                        .borrow_mut()
                        .push(egui::Event::Ime(egui::ImeEvent::Preedit(text)));
                }
            }
        })?;

        listen(&input, "compositionend", &mut listeners, {
            let input = input.clone();
            let pending = Rc::clone(&pending);
            move |event: web_sys::CompositionEvent| {
                if let Some(text) = event.data() {
                    input.set_value("");
                    pending
                        .events
                        .borrow_mut()
                        .push(egui::Event::Ime(egui::ImeEvent::Commit(text)));
                }
            }
        })?;

        // The canvas is sent no keys while this element holds the focus, so winit hears none of
        // these and they are read here: backspace, the arrows, and the rest of what editing a
        // line is made of.
        for (name, pressed) in [("keydown", true), ("keyup", false)] {
            listen(&input, name, &mut listeners, {
                let pending = Rc::clone(&pending);
                move |event: web_sys::KeyboardEvent| {
                    // A key pressed to choose a candidate belongs to the input method, not to the
                    // field. 229 is what a browser reports for one when it says nothing else.
                    if event.is_composing() || event.key_code() == 229 {
                        return;
                    }
                    let modifiers = egui::Modifiers {
                        alt: event.alt_key(),
                        ctrl: event.ctrl_key(),
                        shift: event.shift_key(),
                        mac_cmd: event.meta_key(),
                        command: event.ctrl_key() || event.meta_key(),
                    };
                    pending.modifiers.set(modifiers);
                    let Some(key) = egui::Key::from_name(&event.key()) else {
                        return;
                    };
                    // Otherwise the browser takes the focus off the element and hands it to
                    // whatever it thinks is next on the page.
                    if key == egui::Key::Tab {
                        event.prevent_default();
                    }
                    pending.events.borrow_mut().push(egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers,
                    });
                }
            })?;
        }

        Ok(Self {
            input,
            canvas,
            pending,
            placed: Cell::new(None),
            _listeners: listeners,
        })
    }

    /// Hands the frame everything the element heard since the last one.
    ///
    /// The focus is set as well as the events: winit reports the canvas losing it, which is true
    /// and is not what egui should be told, because it went to this element and the app still has
    /// it.
    pub(crate) fn lend_focus(&self, state: &mut egui_winit::State) {
        let input = state.egui_input_mut();
        let mut events = self.pending.events.borrow_mut();
        if !events.is_empty() {
            input.modifiers = self.pending.modifiers.get();
            input.events.append(&mut events);
        }
        if self.has_focus() {
            input.focused = true;
        }
    }

    /// Follows the caret: takes the focus while there is a field to type into, gives it back to
    /// the canvas when there is not, and stands where the candidate window should open.
    pub(crate) fn follow(&self, ctx: &egui::Context, ime: Option<egui::output::IMEOutput>) {
        match ime {
            Some(_) => {
                if !self.has_focus() {
                    let _ = self.input.focus();
                }
            }
            None => {
                if self.has_focus() {
                    let _ = self.input.blur();
                    let _ = self.canvas.focus();
                }
            }
        }

        if self.placed.get() == ime {
            return;
        }
        self.placed.set(ime);
        let Some(ime) = ime else { return };

        // egui measures in points; the page places in CSS pixels. The zoom factor is the whole of
        // the difference, the device's own pixel ratio being what both sides already agree on.
        let zoom = ctx.zoom_factor();
        let canvas = self.canvas.get_bounding_client_rect();
        let caret = ime.cursor_rect.center();
        let style = self.input.style();
        let _ = style.set_property(
            "left",
            &format!("{}px", canvas.left() as f32 + caret.x * zoom),
        );
        let _ = style.set_property(
            "top",
            &format!("{}px", canvas.top() as f32 + caret.y * zoom),
        );
    }

    fn has_focus(&self) -> bool {
        web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.active_element())
            .is_some_and(|active| active == *self.input.as_ref())
    }
}

impl Drop for TextAgent {
    fn drop(&mut self) {
        self.input.remove();
    }
}

/// Adds one listener and keeps its closure alive in `kept`.
///
/// The closures are typed by the event each one wants, and the DOM only hands out
/// [`web_sys::Event`], so the cast is made here rather than in each of them.
fn listen<E: JsCast>(
    target: &web_sys::HtmlInputElement,
    name: &str,
    kept: &mut Vec<Closure<dyn FnMut(web_sys::Event)>>,
    mut handler: impl FnMut(E) + 'static,
) -> Result<(), JsValue> {
    let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        if let Ok(event) = event.dyn_into::<E>() {
            handler(event);
        }
    });
    target.add_event_listener_with_callback(name, closure.as_ref().unchecked_ref())?;
    kept.push(closure);
    Ok(())
}
