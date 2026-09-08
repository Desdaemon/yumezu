//! The page's input method, which is a hidden text element parked under the caret.
//!
//! winit's web backend has `set_ime_allowed` as an empty function and never sends
//! [`winit::event::WindowEvent::Ime`], so a page has no input method unless one is built out of the
//! browser's own. A browser reports a word being built up only against an element that can be typed
//! into, and a `<canvas>` is not one. So an `<input>` is put on the page, made invisible, moved
//! under wherever egui says the caret is, and given the focus while something is typed. The canvas
//! is then sent no keys, so those are read off the element here too.
//!
//! # Where this comes from
//!
//! Carried from `eframe`, the one piece of it that cannot be depended on instead: the rest of what
//! `eframe` does for an input method is `egui-winit`'s, which [`super::gui`] uses directly, and
//! `eframe` itself wants the window, context and event loop that belong to the 3D renderer.
//!
//! - Upstream: `crates/eframe/src/web/text_agent.rs` at tag `0.36.0`, plus `on_keydown` and
//!   `on_keyup` from `crates/eframe/src/web/events.rs`, the focus half of
//!   `AppRunner::handle_platform_output` in `crates/eframe/src/web/app_runner.rs`, and
//!   `has_focus`, `focus_without_scroll` and `native_pixels_per_point` from
//!   `crates/eframe/src/web/mod.rs`.
//!   <https://github.com/emilk/egui/blob/0.36.0/crates/eframe/src/web/text_agent.rs>
//! - Under MIT OR Apache-2.0, (c) Emil Ernerfeldt and the egui contributors.
//!
//! To take a later version, read what moved with
//!
//! ```notrust
//! git diff 0.36.0..<tag> -- crates/eframe/src/web/text_agent.rs crates/eframe/src/web/events.rs
//! ```
//!
//! and expect the differences below, which are this app's drift rather than upstream's.
//!
//! Two are forced by the app being a version behind upstream, and both come back with the egui
//! bump:
//!
//! - Upstream answers a phone keyboard's corrections -- Gboard offering `Texas` for `tex` -- with
//!   `ImeEvent::DeleteSurrounding`, new in egui 0.36. What is kept here is 0.35's own answer: the
//!   focus bounce in [`Agent::typed`] that stops the suggestion strip appearing at all. Held to
//!   plain typing, and can go when the diff arrives.
//! - Upstream reads `IMEOutput::purpose` to keep a password out of the browser's own
//!   autocompletion. egui 0.35 has no such field, and this app has no password.
//!
//! The rest are the seam:
//!
//! - Events are left on [`egui_winit::State::egui_input_mut`] for the next frame to take, where
//!   upstream pushes them onto its own `AppRunner`.
//! - The canvas is winit's, asked for through `WindowExtWebSys`, not one `eframe` made.
//! - `update` and the focus that upstream keeps in `handle_platform_output` are one call here,
//!   [`TextAgent::follow`], because there is one caller and one place it can be made from.
//! - Nothing asks for a repaint: this app is a render loop around a 3D scene, so the next frame
//!   is already coming, where upstream is drawn on demand.
//! - The keys are translated inline. Upstream's `should_prevent_default_for_key` and
//!   `should_stop_propagation` are dropped: this page has nothing else on it to defend a key from,
//!   so only `Tab` is held back, and only because it would take the focus away. Upstream's
//!   `KeydownSpecialCase` goes with them -- it routes iOS and Android's editing keys into the
//!   `DeleteSurrounding` path that is not here yet.
//! - Upstream's `has_focus` walks to the shadow root the canvas may be inside. This one is on the
//!   page itself, put there by `index.html`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::{JsCast, JsValue, prelude::Closure};

/// Every field is a cell because the whole is shared with every listener: the closures the DOM
/// keeps can only be handed something they own.
struct Agent {
    input: web_sys::HtmlInputElement,
    events: RefCell<Vec<egui::Event>>,
    modifiers: Cell<egui::Modifiers>,
    /// The element's text less whatever is still being composed. The browser only ever reports the
    /// whole line, so this is what says which part of it is new.
    told: RefCell<String>,
}

impl Agent {
    /// Called wherever the element and egui have parted company: an editing key, a browser
    /// reporting a change this cannot read, egui saying it threw the composition away.
    fn clear(&self) {
        self.input.set_value("");
        self.told.borrow_mut().clear();
    }

    fn push(&self, event: egui::Event) {
        self.events.borrow_mut().push(event);
    }

    /// Text landing in the element: a key, a paste, a phone's suggestion, or one step of a word
    /// being composed.
    fn typed(&self, event: &web_sys::InputEvent) {
        let composing = event.is_composing();

        // Only an insertion says anything egui can act on; the element is emptied for the rest of
        // the `inputType` list so the next line starts clean. `insertCompositionText` outside a
        // composition is how the tail of a finished word arrives after `compositionend` has already
        // committed it, so taking it would type the word twice.
        let kind = event.input_type();
        let insertion = kind == "insertText" || kind == "insertReplacementText";
        if !composing && !insertion {
            self.clear();
            return;
        }

        // Clears the suggestion strip a phone keyboard leaves behind. Plain typing only: during a
        // composition this would end the word, and after one the input method's session.
        if !composing {
            let _ = self.input.blur();
            let _ = self.input.focus();
        }

        let text = self.input.value();
        let mut told = self.told.borrow_mut();
        let kept = common_prefix(&text, &told);
        let fresh: String = text.chars().skip(kept).collect();

        if composing {
            self.push(egui::Event::Ime(egui::ImeEvent::Preedit {
                text: fresh,
                active_range_chars: self.active_range(&text, kept),
            }));
            // A word still being built is redrawn whole every time, so only the committed part is
            // remembered: the rest is egui's to replace on the next event.
            *told = text.chars().take(kept).collect();
        } else {
            self.push(egui::Event::Text(fresh));
            *told = text;
        }
    }

    /// Which run of the unfinished word the input method has under consideration, counted in
    /// characters from the start of that word, which is what egui draws apart from the rest.
    ///
    /// The element measures its selection in UTF-16 and egui counts characters. `None` where the
    /// browser cannot be believed -- Android Chrome reports a selection past the end of the value.
    fn active_range(&self, text: &str, kept: usize) -> Option<std::ops::Range<usize>> {
        let start = self.input.selection_start().ok()?? as usize;
        let end = self.input.selection_end().ok()?? as usize;
        let utf16: Vec<u16> = text.encode_utf16().collect();
        if start > end || end > utf16.len() {
            return None;
        }
        let before = String::from_utf16_lossy(&utf16[..start]).chars().count();
        let inside = String::from_utf16_lossy(&utf16[start..end]).chars().count();
        // Counted from the start of the word rather than the start of the line.
        let start = before.saturating_sub(kept);
        Some(start..start + inside)
    }

    /// Whatever the element gained while the word was being composed.
    fn composed(&self) {
        let text = self.input.value();
        let mut told = self.told.borrow_mut();
        let word: String = text.chars().skip(told.chars().count()).collect();
        self.push(egui::Event::Ime(egui::ImeEvent::Commit(word)));
        *told = text;
    }

    /// Every key the app gets while something is typed.
    fn key(&self, event: &web_sys::KeyboardEvent, pressed: bool) {
        // A key choosing a candidate belongs to the input method, not the field, and a browser
        // reports one either by saying so or by reporting 229 and nothing else.
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
        self.modifiers.set(modifiers);

        let name = event.key();
        // A key that is not one character is an editing key rather than text, and so is anything
        // held with a modifier. Either way the element is about to change without reporting it.
        if pressed
            && (name.chars().count() > 1 || modifiers.ctrl || modifiers.alt || modifiers.mac_cmd)
        {
            self.clear();
        }

        let Some(key) = egui::Key::from_name(&name) else {
            return;
        };
        // Otherwise the browser hands the focus to whatever it thinks is next on the page.
        if key == egui::Key::Tab {
            event.prevent_default();
        }
        self.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        });
    }
}

pub(crate) struct TextAgent {
    agent: Rc<Agent>,
    canvas: web_sys::HtmlCanvasElement,
    /// So the element is only moved when the caret has actually moved.
    placed: Cell<Option<egui::output::IMEOutput>>,
    /// Kept only to be kept alive: a listener stops working the moment its closure is dropped.
    _listeners: Vec<Closure<dyn FnMut(web_sys::Event)>>,
}

impl TextAgent {
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
        // A phone would otherwise capitalise the first letter of a world's name.
        input.set_attribute("autocapitalize", "off")?;
        input.set_attribute("aria-hidden", "true")?;

        // Invisible rather than hidden or off-screen: an element the page will not draw is one the
        // browser will not let an input method open against either. Starts over the canvas' top
        // left so focusing it before anything is typed cannot scroll the page elsewhere.
        let style = input.style();
        style.set_property("position", "absolute")?;
        style.set_property("top", &format!("{}px", canvas.offset_top()))?;
        style.set_property("left", &format!("{}px", canvas.offset_left()))?;
        style.set_property("width", "1px")?;
        style.set_property("height", "1px")?;
        style.set_property("border", "none")?;
        style.set_property("outline", "none")?;
        style.set_property("background-color", "transparent")?;
        style.set_property("caret-color", "transparent")?;
        // Under sixteen and a phone browser zooms the page in when the element takes the focus.
        style.set_property("font-size", "16px")?;

        // Next to the canvas rather than at the end of the body, so `position: absolute` resolves
        // against whatever the canvas' own offsets are measured from and the two stay together.
        if let Some(parent) = canvas.parent_node() {
            parent.insert_before(&input, canvas.next_sibling().as_ref())?;
        } else {
            document.body().unwrap().append_child(&input)?;
        }

        let agent = Rc::new(Agent {
            input: input.clone(),
            events: RefCell::default(),
            modifiers: Cell::default(),
            told: RefCell::default(),
        });
        let mut listeners = Vec::new();

        // Every way text arrives, composed or not. Deliberately no `compositionupdate` listener:
        // the element's selection has not been updated yet when that fires.
        listen(&input, "input", &mut listeners, {
            let agent = Rc::clone(&agent);
            move |event: web_sys::InputEvent| agent.typed(&event)
        })?;

        listen(&input, "compositionend", &mut listeners, {
            let agent = Rc::clone(&agent);
            move |_: web_sys::CompositionEvent| agent.composed()
        })?;

        // The canvas is sent no keys while this element holds the focus, so winit hears none of
        // these: backspace, the arrows, and the rest of what editing a line is made of.
        for (name, pressed) in [("keydown", true), ("keyup", false)] {
            listen(&input, name, &mut listeners, {
                let agent = Rc::clone(&agent);
                move |event: web_sys::KeyboardEvent| agent.key(&event, pressed)
            })?;
        }

        Ok(Self {
            agent,
            canvas,
            placed: Cell::new(None),
            _listeners: listeners,
        })
    }

    /// Hands the frame everything the element heard since the last one, and the focus with it:
    /// winit truthfully reports the canvas losing it, but it went to this element.
    pub(crate) fn lend_focus(&self, state: &mut egui_winit::State) {
        let input = state.egui_input_mut();
        let mut events = self.agent.events.borrow_mut();
        if !events.is_empty() {
            input.modifiers = self.agent.modifiers.get();
            input.events.append(&mut events);
        }
        if self.has_focus() {
            input.focused = true;
        }
    }

    /// Takes the focus while there is a field to type into, gives it back to the canvas when there
    /// is not, and stands where the candidate window should open.
    pub(crate) fn follow(&self, ctx: &egui::Context, ime: Option<egui::output::IMEOutput>) {
        match ime {
            Some(ime) => {
                // egui dropped the word being built, so what the element holds is owed to nobody.
                if ime.should_interrupt_composition {
                    self.agent.clear();
                }
                if !self.has_focus() {
                    focus(&self.agent.input);
                }
            }
            None => {
                if self.has_focus() {
                    let _ = self.agent.input.blur();
                    self.agent.clear();
                    focus(&self.canvas);
                }
            }
        }

        if self.placed.get() == ime {
            return;
        }
        self.placed.set(ime);
        let Some(ime) = ime else { return };

        // egui measures in points and the page places in CSS pixels; the zoom factor is the whole
        // of the difference, both sides already agreeing on the device's pixel ratio. Offsets
        // rather than a bounding rect, to measure from the same corner `position: absolute` does.
        // Clamped inside the canvas so a caret scrolled out of sight cannot scroll the page to it.
        let zoom = ctx.zoom_factor();
        let ratio = pixel_ratio();
        let caret = ime.cursor_rect.center();
        let x = (caret.x * zoom).clamp(0.0, self.canvas.width() as f32 / ratio);
        let y = (caret.y * zoom).clamp(0.0, self.canvas.height() as f32 / ratio);
        let style = self.agent.input.style();
        let _ = style.set_property("left", &format!("{}px", self.canvas.offset_left() as f32 + x));
        let _ = style.set_property("top", &format!("{}px", self.canvas.offset_top() as f32 + y));
    }

    fn has_focus(&self) -> bool {
        web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.active_element())
            .is_some_and(|active| active == *self.agent.input.as_ref())
    }
}

impl Drop for TextAgent {
    fn drop(&mut self) {
        self.agent.input.remove();
    }
}

fn common_prefix(a: &str, b: &str) -> usize {
    std::iter::zip(a.chars(), b.chars())
        .take_while(|(a, b)| a == b)
        .count()
}

/// Without scrolling it into view, which on an embedded page would scroll the page.
fn focus(element: &web_sys::HtmlElement) {
    let options = web_sys::FocusOptions::new();
    options.set_prevent_scroll(true);
    let _ = element.focus_with_options(&options);
}

/// The device's own pixels per CSS pixel, which is what the canvas' size is counted in.
fn pixel_ratio() -> f32 {
    let ratio = web_sys::window().unwrap().device_pixel_ratio() as f32;
    if ratio > 0.0 && ratio.is_finite() { ratio } else { 1.0 }
}

/// Keeps the closure alive in `kept`. Handlers are typed by the event each wants and the DOM only
/// hands out [`web_sys::Event`], so the cast is made here rather than in each of them.
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
