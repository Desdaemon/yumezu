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
//! and expect the differences below, which are this app's rather than upstream's drift.
//!
//! Two of them are the crate this is vendored *into* rather than choices, because the app is a
//! version behind upstream and egui 0.35 cannot say what 0.36's agent says. Both come back with
//! the egui bump:
//!
//! - Upstream answers a phone keyboard's corrections -- Gboard offering `Texas` for `tex` -- by
//!   diffing the element against what egui was last told and sending
//!   `ImeEvent::DeleteSurrounding` for the difference. That variant is new in egui 0.36, so what
//!   is kept here instead is 0.35's own answer, the focus bounce in [`Agent::typed`] that stops
//!   the suggestion strip appearing at all. It is held to plain typing, where it was always
//!   aimed, and can go when the diff arrives.
//! - Upstream reads `IMEOutput::purpose` to keep a password out of the browser's own
//!   autocompletion. egui 0.35 has no such field, and this app has no password.
//!
//! The rest are the seam:
//!
//! - Events are left on [`egui_winit::State::egui_input_mut`] for the next frame to take, where
//!   upstream pushes them onto its own `AppRunner`. That is the whole of it.
//! - The canvas is winit's, asked for through `WindowExtWebSys`, not one `eframe` made.
//! - `update` and the focus that upstream keeps in `handle_platform_output` are one call here,
//!   [`TextAgent::follow`], because there is one caller and one place it can be made from.
//! - Nothing asks for a repaint. Upstream is drawn on demand and has to; this app is a render
//!   loop around a 3D scene, so the next frame is already coming.
//! - The keys are translated inline. Upstream's `should_prevent_default_for_key` and its
//!   `should_stop_propagation` option are dropped: this page has nothing else on it to defend a
//!   key from, so only `Tab` is held back, and only because it would take the focus away.
//!   Upstream's `KeydownSpecialCase` goes with them -- it exists to route iOS and Android's
//!   editing keys into the `DeleteSurrounding` path that is not here yet.
//! - Upstream's `has_focus` walks to the shadow root the canvas may be inside. This one is on the
//!   page itself, put there by `index.html`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::{JsCast, JsValue, prelude::Closure};

/// The element, what it has heard since the last frame, and how much of what it holds egui has
/// already been told about.
///
/// Shared with every listener, which is why each part of it is a cell: the closures the DOM keeps
/// can only be handed something they own.
struct Agent {
    input: web_sys::HtmlInputElement,
    events: RefCell<Vec<egui::Event>>,
    modifiers: Cell<egui::Modifiers>,
    /// The text egui has already been given, which is the element's own text less whatever is
    /// still being composed. What arrives is only ever reported as the whole line, so this is
    /// what says which part of that line is new.
    told: RefCell<String>,
}

impl Agent {
    /// Starts the line again, with nothing in the element and nothing owed to egui.
    ///
    /// Called wherever the two have parted company: an editing key, a browser reporting a change
    /// this cannot read, egui saying it has thrown the composition away.
    fn clear(&self) {
        self.input.set_value("");
        self.told.borrow_mut().clear();
    }

    fn push(&self, event: egui::Event) {
        self.events.borrow_mut().push(event);
    }

    /// Text landing in the element, which is every way a browser has of saying something was
    /// typed: a key, a paste, a phone's suggestion, and each step of a word being composed.
    fn typed(&self, event: &web_sys::InputEvent) {
        let composing = event.is_composing();

        // Only an insertion says something egui can act on, and only while it is either part of a
        // composition or the plain typing that is not one. Everything else in the `inputType`
        // list is dropped, and the element emptied so that the next line starts clean.
        //
        // `insertCompositionText` outside a composition is the one that matters: it is how the
        // tail end of a finished word arrives, after `compositionend` has already committed it.
        // Taking it would type the word twice.
        let kind = event.input_type();
        let insertion = kind == "insertText" || kind == "insertReplacementText";
        if !composing && !insertion {
            self.clear();
            return;
        }

        // Clears the suggestion strip a phone keyboard leaves behind, by taking the focus off the
        // element and putting it straight back. Plain typing only: during a composition this
        // would end the word, and after one it would end the input method's session.
        if !composing {
            let _ = self.input.blur();
            let _ = self.input.focus();
        }

        let text = self.input.value();
        let mut told = self.told.borrow_mut();
        // What is new is whatever follows the part egui already has.
        let kept = common_prefix(&text, &told);
        let fresh: String = text.chars().skip(kept).collect();

        if composing {
            self.push(egui::Event::Ime(egui::ImeEvent::Preedit {
                text: fresh,
                active_range_chars: self.active_range(&text, kept),
            }));
            // A word still being built is redrawn whole every time, so only the part already
            // committed is remembered: the rest is egui's to replace on the next event.
            *told = text.chars().take(kept).collect();
        } else {
            self.push(egui::Event::Text(fresh));
            *told = text;
        }
    }

    /// Which run of the unfinished word the input method has under consideration, counted in
    /// characters from the start of that word. egui draws it apart from the rest.
    ///
    /// The element measures its selection in UTF-16, and egui counts characters, so the text
    /// either side of the selection is converted back and counted. `None` where the browser
    /// cannot be believed -- Android Chrome reports a selection past the end of the value.
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

    /// The finished word, which is whatever the element gained while it was being composed.
    fn composed(&self) {
        let text = self.input.value();
        let mut told = self.told.borrow_mut();
        let word: String = text.chars().skip(told.chars().count()).collect();
        self.push(egui::Event::Ime(egui::ImeEvent::Commit(word)));
        *told = text;
    }

    /// A key on the element, which is every key the app gets while something is being typed.
    fn key(&self, event: &web_sys::KeyboardEvent, pressed: bool) {
        // A key pressed to choose a candidate belongs to the input method, not to the field, and
        // a browser reports one either by saying so or by reporting 229 and nothing else.
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
        // held down with a modifier. Either way the line is about to be changed by something the
        // element will not report, so what it holds is no longer what egui holds.
        if pressed
            && (name.chars().count() > 1 || modifiers.ctrl || modifiers.alt || modifiers.mac_cmd)
        {
            self.clear();
        }

        let Some(key) = egui::Key::from_name(&name) else {
            return;
        };
        // Otherwise the browser takes the focus off the element and hands it to whatever it
        // thinks is next on the page.
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
        // an element the browser will not let an input method open against. It starts over the
        // canvas' top left so that focusing it before anything is typed cannot scroll some other
        // part of the page into view, and is moved under the caret from [`Self::follow`].
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

        // Next to the canvas rather than at the end of the body, so that `position: absolute`
        // resolves against whatever the canvas' own offsets are measured from. The two then stay
        // together however the page is scrolled or the canvas embedded.
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

        // Every way text arrives, composed or not. There is deliberately no `compositionupdate`
        // listener: the word so far is read here instead, because the element's selection -- what
        // says which part of the word the input method is working on -- has not been updated yet
        // when `compositionupdate` is raised. Nor is there a `compositionstart` one, which
        // upstream keeps only to ask for a repaint.
        listen(&input, "input", &mut listeners, {
            let agent = Rc::clone(&agent);
            move |event: web_sys::InputEvent| agent.typed(&event)
        })?;

        listen(&input, "compositionend", &mut listeners, {
            let agent = Rc::clone(&agent);
            move |_: web_sys::CompositionEvent| agent.composed()
        })?;

        // The canvas is sent no keys while this element holds the focus, so winit hears none of
        // these and they are read here: backspace, the arrows, and the rest of what editing a
        // line is made of.
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

    /// Hands the frame everything the element heard since the last one.
    ///
    /// The focus is set as well as the events: winit reports the canvas losing it, which is true
    /// and is not what egui should be told, because it went to this element and the app still has
    /// it.
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

    /// Follows the caret: takes the focus while there is a field to type into, gives it back to
    /// the canvas when there is not, and stands where the candidate window should open.
    pub(crate) fn follow(&self, ctx: &egui::Context, ime: Option<egui::output::IMEOutput>) {
        match ime {
            Some(ime) => {
                // egui has dropped the word being built -- the field lost the focus, or its text
                // was replaced from elsewhere -- so what the element still holds is owed to
                // nobody.
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

        // egui measures in points and the page places in CSS pixels, and the zoom factor is the
        // whole of the difference -- the device's own pixel ratio is what both sides already
        // agree on. Offsets rather than a bounding rect, to be measured from the same corner the
        // element's `position: absolute` is, and held inside the canvas so that putting the
        // element under a caret scrolled out of sight cannot scroll the page to it.
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

/// How many characters two lines open with in common.
fn common_prefix(a: &str, b: &str) -> usize {
    std::iter::zip(a.chars(), b.chars())
        .take_while(|(a, b)| a == b)
        .count()
}

/// Gives an element the focus without scrolling it into view, which on a page the app is embedded
/// in would scroll the page.
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
