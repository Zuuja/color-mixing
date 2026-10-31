//! The browser layer: the only file that talks to the DOM.
//!
//! Every decision — what a round asks, when a check counts, what the hints
//! say, what is worth storing — was made in [`crate::mix`] and is unit tested
//! there. What is left here is `get_element_by_id`, `set_text_content`,
//! `add_event_listener`, and nothing that could have been written down as a
//! rule.
//!
//! The entry point is `#[wasm_bindgen(start)]`, so the loader in `ui.html`
//! needs no argument list, no callback name and no glue: importing the module
//! runs the app.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{Document, Element, HtmlButtonElement, HtmlElement, Window};

use crate::mix::{self, Game, Settings};

/// The app's mutable state: the game, plus the seed the next round is dealt
/// from.
///
/// `Rc<RefCell<_>>` because wasm has no threads and the event handlers below
/// are plain closures, not `'static` Rust threads. The rule that keeps this
/// honest: a handler borrows, mutates, drops the borrow, and then renders. It
/// never renders from inside a borrow.
struct State {
    game: Game,
    seed: u64,
}

type Shared = Rc<RefCell<State>>;

/// The scope this app's worker is registered for.
///
/// Stated rather than inherited, and it is the one string that has to agree
/// with `src/service-worker.js`: the worker resolves its own directory the same
/// way, from `self.location`. See [`announce_offline`] for why this is not
/// optional.
const SCOPE: &str = "./";

/// Storage keys, one prefix per app so sibling PWAs on this origin cannot
/// read or overwrite each other's values.
const SETTINGS_KEY: &str = "color_mix.settings";
const SCORE_KEY: &str = "color_mix.score";

thread_local! {
    /// The last rendered picks signature, so a parts change rewrites a count
    /// instead of rebuilding the row under the player's thumb.
    static LAST_PICKS: RefCell<String> = const { RefCell::new(String::new()) };
    /// The last rendered palette, as its own id list. Rebuilt only when the
    /// round offers different swatches.
    static LAST_PALETTE: RefCell<String> = const { RefCell::new(String::new()) };
    /// The last rendered settings ranges, so the option lists are rebuilt
    /// only when the mix shape moves their floors.
    static LAST_RANGES: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Start the app.
///
/// Runs automatically when the page's dynamic import resolves, because the
/// loader in `ui.html` is `import('./app.js').then(m => m.default())` and
/// `default` is this start function.
///
/// # Errors
///
/// When there is no window or no document to build the app in — a headless
/// or non-browser context. The rejection is what the loader's `.catch`
/// reports; there is nothing else to do with it.
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("no document"))?;

    let state: Shared = Rc::new(RefCell::new(load(&window)));

    render(&document, &state);
    bind(&window, &document, &state);
    // Owned clones, because the spawned future outlives this function.
    let offline: Window = window.clone();
    spawn_local(announce_offline(offline));
    Ok(())
}

/// Read the persisted game, falling back to a fresh one.
///
/// A `localStorage` that throws — Safari in private mode, a browser with site
/// data blocked — is a normal condition, not an error worth failing startup
/// over. The settings and score parsers handle everything else.
fn load(window: &Window) -> State {
    let storage = window.local_storage().ok().flatten();
    let read = |key: &str| -> Option<String> {
        storage
            .as_ref()
            .and_then(|storage| storage.get_item(key).ok().flatten())
    };
    let settings = Settings::from_stored(read(SETTINGS_KEY).as_deref());
    let mut game = Game::new(settings, seed_now());
    game.score = mix::Score::from_stored(read(SCORE_KEY).as_deref());
    State {
        game,
        seed: seed_now(),
    }
}

/// A seed for the first round, from the wall clock's raw bits.
///
/// No dependency, no float-to-integer cast to lint: the bit pattern of a
/// millisecond timestamp is quite random enough to deal rounds, and the seed
/// only has to change between visits.
fn seed_now() -> u64 {
    js_sys::Date::now().to_bits()
}

/// Write the settings, or clear the key when they are the defaults.
fn store_settings(window: &Window, settings: &Settings) {
    let Some(storage) = window.local_storage().ok().flatten() else {
        return;
    };
    let result = match settings.to_stored() {
        Some(value) => storage.set_item(SETTINGS_KEY, &value),
        None => storage.remove_item(SETTINGS_KEY),
    };
    if let Err(error) = result {
        warn("could not save the settings", &error);
    }
}

/// Write the score, or clear the key while nothing has been scored.
fn store_score(window: &Window, state: &State) {
    let Some(storage) = window.local_storage().ok().flatten() else {
        return;
    };
    let key = SCORE_KEY;
    let result = match state.game.score.to_stored() {
        Some(value) => storage.set_item(key, &value),
        None => storage.remove_item(key),
    };
    if let Err(error) = result {
        warn("could not save the score", &error);
    }
}

/// Draw the current state into the shell.
///
/// Every text node, attribute and disabled flag this app owns is written
/// here, from the state, on every change. Nothing in the DOM is a source of
/// truth. The two dynamic lists — the palette and the picks — are rebuilt
/// only when their shape changes, so a tap never rebuilds the row it
/// happened in.
fn render(document: &Document, state: &Shared) {
    let game = state.borrow();
    let game = &game.game;

    // The round's ask and the running score.
    set_text(document, "round-line", &game.round_line());
    set_text(document, "score-line", &game.score_line());

    // The target is always shown; the attempt only when the reveal rule says
    // so. The checkerboard sits under the colour, so hiding is just clearing
    // the inline background.
    paint(
        document,
        "target-swatch",
        &mix::hex(game.round.target_rgb()),
    );
    let reveal = game.reveal_attempt();
    let attempt_element: Option<HtmlElement> =
        element(document, "attempt-swatch").and_then(|node| node.dyn_into().ok());
    if let Some(swatch) = &attempt_element {
        let shown = match game.attempt_rgb() {
            Some(rgb) if reveal => mix::hex(rgb),
            _ => String::new(),
        };
        set_style(swatch, "background", &shown);
        set_attribute(
            swatch,
            "data-shown",
            if shown.is_empty() { "false" } else { "true" },
        );
    }
    let note = if reveal { "" } else { "hidden until you check" };
    set_text(document, "attempt-note", note);

    // The one status line, and its two moods.
    let status = element(document, "status");
    if let Some(status) = &status {
        set_text(document, "status", &game.status());
        // Neutral gets neither mood: an ordinary instruction must not read
        // as an error, and a solve must not read as one either.
        let good = matches!(game.outcome, Some(mix::Outcome::Solved(_)));
        let bad = matches!(game.outcome, Some(mix::Outcome::Wrong));
        set_attribute(status, "data-good", if good { "true" } else { "false" });
        set_attribute(status, "data-bad", if bad { "true" } else { "false" });
    }

    // The hints the round has given away, one line each.
    set_text(document, "hints", &game.round.hint_lines().join("  ·  "));
    // And how the parts divide right now, under the strip.
    set_text(document, "parts-line", &parts_line(game));

    render_picks(document, game);
    render_palette(document, game);

    // Buttons: `disabled` is the honest way to say "this cannot act now".
    let check = button(document, "check-button");
    if let Some(check) = &check {
        // Check is disabled only while nothing is picked: any attempt —
        // half-filled, over-filled, whatever — gets a verdict. The gate that
        // demanded the round's exact part count was what read as "checking
        // is broken".
        check.set_disabled(game.attempt.is_empty());
    }
    let hint = button(document, "hint-button");
    if let Some(hint) = &hint {
        // With everything revealed, there is nothing left to ask for.
        let exhausted = game.round.revealed.len() == game.round.ingredients.len() + 1;
        hint.set_disabled(exhausted);
    }
    let next = button(document, "next-button");
    if let Some(next) = &next {
        set_text_element(next, game.next_label());
    }

    render_settings(document, game);
}

/// The picks list: one row per picked colour, each with its parts stepper.
///
/// Rebuilt only when the picks' signature changes — a parts change rewrites a
/// row's count but not the row, so the button under the player's thumb never
/// moves.
fn render_picks(document: &Document, game: &Game) {
    let Some(host) = element(document, "picks") else {
        return;
    };

    let signature: String = game
        .attempt
        .iter()
        .map(|entry| entry.pigment.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let seen = LAST_PICKS.with(|last| last.borrow().clone());
    if seen != signature {
        LAST_PICKS.with(|last| last.borrow_mut().clone_from(&signature));
        clear(&host);
        for entry in &game.attempt {
            let pigment = &mix::PIGMENTS[entry.pigment];
            // The chip itself removes the colour: one big, colour-identified
            // tap target, no separate destroy button to hunt for.
            let Some(chip) = chip(document, entry.pigment) else {
                return;
            };
            let Some(share) = span(document, "share") else {
                return;
            };
            share.set_text_content(Some(&entry.parts.to_string()));
            set_style(&share, "color", mix::text_on(pigment.rgb));
            let Some(cross) = span(document, "cross") else {
                return;
            };
            cross.set_text_content(Some("\u{00d7}"));
            set_style(&cross, "color", mix::text_on(pigment.rgb));
            let _ = chip.append_child(&share);
            let _ = chip.append_child(&cross);
            let _ = host.append_child(&chip);
        }
        return;
    }

    // Same chips, new counts: rewrite only the numbers.
    let chips = host.children();
    for index in 0..chips.length() {
        let (Some(chip), Some(entry)) = (chips.item(index), game.attempt.get(index as usize))
        else {
            continue;
        };
        let Some(pigment) = chip
            .get_attribute("data-id")
            .and_then(|text| text.parse::<usize>().ok())
            .map(|id| &mix::PIGMENTS[id])
        else {
            continue;
        };
        let parts = chip.children();
        // The chip is share then cross; the count is the first.
        let Some(share) = parts.item(0) else {
            continue;
        };
        share.set_text_content(Some(&entry.parts.to_string()));
        let typed: Option<HtmlElement> = share.dyn_into().ok();
        if let Some(share) = typed {
            set_style(&share, "color", mix::text_on(pigment.rgb));
        }
    }
}

/// One pick chip: a button carrying the pigment id, filled with the colour.
fn chip(document: &Document, pigment: usize) -> Option<HtmlButtonElement> {
    let button: HtmlButtonElement = node(document, "button", "pick")?.dyn_into().ok()?;
    let _ = button.set_attribute("type", "button");
    let _ = button.set_attribute("data-id", &pigment.to_string());
    set_style(&button, "background", &mix::hex(mix::PIGMENTS[pigment].rgb));
    Some(button)
}

/// The words under the strip: how the attempt stands, as counts against the
/// round — "3/2 colours, 8/6 parts" — then the split by name.
///
/// Both counts are attempt/round. Colours over the round's count are the
/// player exploring; parts over mean the same. The words say so once, here,
/// instead of the numbers lying by omission.
fn parts_line(game: &Game) -> String {
    // Hidden numbers: the line lists the mix by name and part only — the
    // dealt colour count, the dealt part count and the remaining room all
    // stay secret, because the player is meant to judge by eye.
    if game.settings.up_to && !game.settings.reveal_numbers {
        if game.attempt.is_empty() {
            return String::new();
        }
        let mut listed: Vec<String> = game
            .attempt
            .iter()
            .map(|entry| format!("{} {}", entry.parts, mix::PIGMENTS[entry.pigment].name))
            .collect();
        listed.sort();
        return format!("Your mix: {}", listed.join(", "));
    }
    let colours = format!(
        "{}/{} colours",
        game.attempt.len(),
        game.round.ingredients.len()
    );
    if game.attempt.is_empty() {
        return format!("{colours}, 0/{} parts", game.round.parts_used);
    }
    let mut listed: Vec<String> = game
        .attempt
        .iter()
        .map(|entry| format!("{} {}", entry.parts, mix::PIGMENTS[entry.pigment].name))
        .collect();
    listed.sort();
    let placed = game.assigned();
    let total = game.round.parts_used;
    // Ceilings mode never calls the remainder a requirement: "to place"
    // becomes "of room left".
    let suffix = match (placed.cmp(&total), game.settings.up_to) {
        (std::cmp::Ordering::Equal, _) => String::new(),
        (std::cmp::Ordering::Less, false) => format!(" — {} more to place", total - placed),
        (std::cmp::Ordering::Less, true) => {
            format!(" — {} of room left", total - placed)
        }
        (std::cmp::Ordering::Greater, _) => format!(" — {} over", placed - total),
    };
    format!(
        "{colours}, {placed}/{} parts: {}{}",
        game.round.parts_used,
        listed.join(", "),
        suffix
    )
}

/// The palette: one button per swatch the round offers.
///
/// Rebuilt only when the round's palette changes; the picked and hinted
/// marks are attributes rewritten in place on every render, so a tap toggles
/// a ring without moving a single button.
fn render_palette(document: &Document, game: &Game) {
    let Some(host) = element(document, "palette") else {
        return;
    };

    let signature: String = game
        .round
        .palette
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let seen = LAST_PALETTE.with(|last| last.borrow().clone());
    if seen != signature {
        LAST_PALETTE.with(|last| last.borrow_mut().clone_from(&signature));
        clear(&host);
        for &id in &game.round.palette {
            let pigment = &mix::PIGMENTS[id];
            let Some(swatch) = node(document, "button", "pigment") else {
                return;
            };
            let _ = swatch.set_attribute("type", "button");
            let _ = swatch.set_attribute("data-id", &id.to_string());
            let Some(chip) = span(document, "chip") else {
                return;
            };
            set_style(&chip, "background", &mix::hex(pigment.rgb));
            let _ = swatch.append_child(&chip);
            let Some(label) = span(document, "name") else {
                return;
            };
            label.set_text_content(Some(pigment.name));
            let _ = swatch.append_child(&label);
            let _ = host.append_child(&swatch);
        }
    }

    // Marks, rewritten every render from the state alone.
    let buttons = host.children();
    for index in 0..buttons.length() {
        let Some(node) = buttons.item(index) else {
            continue;
        };
        let Some(id) = node
            .get_attribute("data-id")
            .and_then(|text| text.parse::<usize>().ok())
        else {
            continue;
        };
        let picked = game.attempt.iter().find(|entry| entry.pigment == id);
        let hinted = game
            .round
            .revealed
            .iter()
            .any(|hint| matches!(hint, mix::Hint::Ingredient(ingredient) if *ingredient == id));
        let _ = node.set_attribute(
            "data-picked",
            if picked.is_some() { "true" } else { "false" },
        );
        let _ = node.set_attribute("data-hinted", if hinted { "true" } else { "false" });
        // The picked swatch carries its part count as a badge, so the share
        // is visible where the player taps to add more.
        if let Some(old_badge) = node.children().item(2) {
            let _ = node.remove_child(&old_badge);
        }
        if let Some(entry) = picked {
            let Some(badge) = span(document, "badge") else {
                return;
            };
            badge.set_text_content(Some(&entry.parts.to_string()));
            let _ = node.append_child(&badge);
        }
    }
}

/// The settings selects: option lists derived from the mix shape, values set
/// from the state.
///
/// The option lists only change when `mix_parts` does — a bigger mix raises
/// the floor under both other numbers — so they are rebuilt then and only
/// then. Every render re-asserts the values, so the screen never disagrees
/// with the state after a clamped change.
fn render_settings(document: &Document, game: &Game) {
    let settings = &game.settings;
    let floor = settings.mix_parts;

    let rebuild = |document: &Document, id: &str, from: usize, to: usize| -> Option<()> {
        let select = select_element(document, id)?;
        let signature = format!("{id}:{from}..{to}");
        let seen = LAST_RANGES.with(|last| {
            let mut last = last.borrow_mut();
            if last.contains(&signature) {
                true
            } else {
                last.push_str(&signature);
                false
            }
        });
        if seen {
            return Some(());
        }
        // Rebuilding options mid-pick would yank the list from under the
        // reader, so the signature check above is the only path here.
        clear(&select);
        for value in from..=to {
            let option = document.create_element("option").ok()?;
            let _ = option.set_attribute("value", &value.to_string());
            option.set_text_content(Some(&value.to_string()));
            let _ = select.append_child(&option);
        }
        Some(())
    };

    // The swatch count runs from one more than the mix to the whole palette.
    rebuild(document, "color-count", floor + 1, mix::MAX_COLORS);
    // The parts split runs from one more than the mix to the ceiling.
    rebuild(document, "parts-total", floor + 1, mix::MAX_PARTS);
    // The mix itself runs from the smallest mix to the largest.
    rebuild(document, "mix-parts", mix::MIN_MIX, mix::MAX_MIX);

    for (id, value) in [
        ("color-count", settings.color_count),
        ("mix-parts", settings.mix_parts),
        ("parts-total", settings.parts_total),
    ] {
        if let Some(select) = select_element(document, id) {
            select.set_value(&value.to_string());
        }
    }
    if let Some(box_) = input_element(document, "show-preview") {
        box_.set_checked(settings.show_preview);
    }
    if let Some(box_) = input_element(document, "up-to") {
        box_.set_checked(settings.up_to);
    }
    if let Some(box_) = input_element(document, "hide-numbers") {
        box_.set_checked(!settings.reveal_numbers);
    }
    // Hiding numbers only means something in ceilings mode: grey the row out
    // in every other shape, so the control never looks broken.
    if let Some(row) = element(document, "hide-numbers-row") {
        let _ = row.set_attribute("data-off", if settings.up_to { "false" } else { "true" });
    }
}

/// Bind the controls, once, at start.
///
/// The palette and the picks use one delegated listener each: their rows are
/// rebuilt as the game changes, but the containers are permanent, so the
/// closure never has to be re-hung. Handlers mutate the state and then call
/// `render`. That is the whole application loop; there is no framework, no
/// virtual DOM and no diffing.
fn bind(window: &Window, document: &Document, state: &Shared) {
    let window: Rc<Window> = Rc::new(window.clone());
    let document: Rc<Document> = Rc::new(document.clone());

    // Palette: a tap toggles the pigment in or out of the attempt.
    on_click(&document, "palette", {
        let document = Rc::clone(&document);
        let state = Rc::clone(state);
        move |target| {
            if let Some(id) = data_id(&target) {
                let mut state_ref = state.borrow_mut();
                if state_ref
                    .game
                    .attempt
                    .iter()
                    .any(|entry| entry.pigment == id)
                {
                    // Already picked: give it one more part, while parts remain.
                    state_ref.game.nudge(id, true);
                } else {
                    state_ref.game.toggle(id);
                }
                drop(state_ref);
                render(&document, &state);
            }
        }
    });

    // Picks: tapping a chip takes one part back. At one part the colour
    // leaves with it — one tap, one unit, never the whole colour at once.
    on_click(&document, "picks", {
        let document = Rc::clone(&document);
        let state = Rc::clone(state);
        move |target| {
            if let Some(id) = data_id(&target) {
                state.borrow_mut().game.take_back(id);
                render(&document, &state);
            }
        }
    });

    // Hint: pay, reveal, render, store.
    on_click(&document, "hint-button", {
        let window = Rc::clone(&window);
        let document = Rc::clone(&document);
        let state = Rc::clone(state);
        move |_| {
            let spent = state.borrow_mut().game.hint();
            if spent.is_some() {
                let state_ref = state.borrow();
                store_score(&window, &state_ref);
                drop(state_ref);
                render(&document, &state);
            }
        }
    });

    // Check: judge the attempt, score it, store, render.
    on_click(&document, "check-button", {
        let window = Rc::clone(&window);
        let document = Rc::clone(&document);
        let state = Rc::clone(state);
        move |_| {
            let judged = state.borrow_mut().game.check();
            if judged.is_some() {
                let state_ref = state.borrow();
                store_score(&window, &state_ref);
                drop(state_ref);
                render(&document, &state);
            }
        }
    });

    // Next: deal the next round.
    on_click(&document, "next-button", {
        let document = Rc::clone(&document);
        let state = Rc::clone(state);
        move |_| {
            {
                let mut state_ref = state.borrow_mut();
                state_ref.seed = state_ref.seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
                let seed = state_ref.seed;
                state_ref.game.next_round(seed);
            }
            render(&document, &state);
        }
    });

    // Settings: any of the four controls re-shapes the game and deals fresh.
    for id in ["color-count", "mix-parts", "parts-total"] {
        on_change(&document, id, {
            let window = Rc::clone(&window);
            let document = Rc::clone(&document);
            let state = Rc::clone(state);
            move |_| {
                apply_settings_from_controls(&window, &document, &state);
            }
        });
    }
    on_change(&document, "show-preview", {
        let window = Rc::clone(&window);
        let document = Rc::clone(&document);
        let state = Rc::clone(state);
        move |_| {
            apply_settings_from_controls(&window, &document, &state);
        }
    });
    on_change(&document, "up-to", {
        let window = Rc::clone(&window);
        let document = Rc::clone(&document);
        let state = Rc::clone(state);
        move |_| {
            apply_settings_from_controls(&window, &document, &state);
        }
    });
    on_change(&document, "hide-numbers", {
        let window = Rc::clone(&window);
        let document = Rc::clone(&document);
        let state = Rc::clone(state);
        move |_| {
            apply_settings_from_controls(&window, &document, &state);
        }
    });
}

/// Read the four controls and apply them as the game's new settings.
///
/// The selects are the source here, read once and handed to the clamps; a
/// mismatched or empty value falls back to the current setting unchanged.
fn apply_settings_from_controls(window: &Window, document: &Document, state: &Shared) {
    let read =
        |id: &str| -> Option<usize> { select_element(document, id)?.value().parse::<usize>().ok() };
    let preview = input_element(document, "show-preview").is_some_and(|box_| box_.checked());
    let up_to = input_element(document, "up-to").is_some_and(|box_| box_.checked());
    let hide = input_element(document, "hide-numbers").is_some_and(|box_| box_.checked());
    let current = state.borrow().game.settings;
    let settings = Settings::full(
        read("color-count").unwrap_or(current.color_count),
        read("mix-parts").unwrap_or(current.mix_parts),
        read("parts-total").unwrap_or(current.parts_total),
        preview,
        up_to,
        !hide,
    );
    // Only the three numbers change the round's shape. A settings change
    // that did not touch them must not deal a fresh round — the player's
    // attempt is not the settings panel's to throw away.
    let current_triple = (current.color_count, current.mix_parts, current.parts_total);
    let next_triple = (
        settings.color_count,
        settings.mix_parts,
        settings.parts_total,
    );
    if current_triple != next_triple {
        let mut state_ref = state.borrow_mut();
        state_ref.seed = state_ref.seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let seed = state_ref.seed;
        state_ref.game.apply_settings(settings, seed);
    } else {
        // Same shape: apply in place, keeping round and attempt.
        state.borrow_mut().game.settings = settings;
    }
    store_settings(window, &state.borrow().game.settings);
    render(document, state);
}

/// Hang a click listener on one element, delegated or not.
///
/// The handler receives the clicked element when it is (or is inside) the
/// host, already cast. `forget` is the correct owner here: the closure must
/// outlive `bind` or the listener becomes a dangling function pointer.
fn on_click<F>(document: &Document, host_id: &str, handler: F)
where
    F: FnMut(HtmlElement) + 'static,
{
    let Some(host) = element(document, host_id) else {
        return;
    };
    let mut handler = handler;
    let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        let Some(target) = event.target() else {
            return;
        };
        let Some(element) = target.dyn_into::<HtmlElement>().ok() else {
            return;
        };
        handler(element);
    });
    if let Err(error) =
        host.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref())
    {
        warn("could not bind a click", &error);
        return;
    }
    closure.forget();
}

/// Hang a change listener on one element.
fn on_change<F>(document: &Document, host_id: &str, handler: F)
where
    F: FnMut(web_sys::Event) + 'static,
{
    let Some(host) = element(document, host_id) else {
        return;
    };
    let closure = Closure::<dyn FnMut(web_sys::Event)>::new(handler);
    if let Err(error) =
        host.add_event_listener_with_callback("change", closure.as_ref().unchecked_ref())
    {
        warn("could not bind a change", &error);
        return;
    }
    closure.forget();
}

/// The `data-id` attribute of the tapped element, or of its nearest ancestor
/// that carries one.
///
/// Delegation hands the deepest node to the handler — the chip span inside a
/// palette button, most often — so the walk up is the whole trick.
fn data_id(target: &HtmlElement) -> Option<usize> {
    let mut node: Option<Element> = Some(target.clone().into());
    while let Some(current) = node {
        if let Some(text) = current.get_attribute("data-id") {
            return text.parse::<usize>().ok();
        }
        node = current.parent_element();
    }
    None
}

/// Register the service worker, and release any stale registration.
///
/// Deliberately silent. The worker is a background concern: either it works
/// and the user notices nothing, or it does not and the app still runs.
/// Nothing is lost that the reader can act on.
///
/// Registration is awaited, but not before the app is interactive: offline
/// support is not worth a blank screen.
async fn announce_offline(window: Window) {
    let container = window.navigator().service_worker();

    // The scope is stated rather than inherited. Left to itself, a registration's
    // scope is the directory of the page that registered it: right today, and
    // silently wrong once the app is published somewhere else or opened
    // through a path resolving higher up the origin. A worker registered for
    // the whole origin answers for every page on it, including pages that have
    // nothing to do with this app. Naming the scope keeps that claim as small
    // as the app.
    let options = web_sys::RegistrationOptions::new();
    options.set_scope(SCOPE);
    let registered =
        JsFuture::from(container.register_with_options("./service-worker.js", &options)).await;
    let registered = match registered {
        Ok(value) => value,
        Err(error) => {
            warn("service worker registration failed", &error);
            return;
        }
    };

    // The promise resolves to the registration, which is the handle the
    // update nudge watches.
    if let Ok(registered) = registered.dyn_into::<web_sys::ServiceWorkerRegistration>() {
        nudge_worker_update(&registered).await;
    }

    release_stale_registrations(&window).await;
}

/// Ask this app's service worker to update as soon as it can take over.
///
/// The worker never swaps itself in unasked: an update waits for every page
/// it controls to close, because a live app must not run half old code under
/// half new. Left exactly there, an installed copy would update only when
/// every opening of it has closed — on a phone, possibly never. So this
/// asks, and the ask is safe by construction: the sender is the page itself,
/// which then reloads as the swap happens. The worker answers the message
/// with `skipWaiting`, the browser fires `controllerchange`, the page reloads
/// exactly once, and the old worker is redundant. No client ever straddles
/// the two versions — the one that asked is the one that stepped aside.
///
/// The message is sent for a worker already installed and waiting, and for
/// one still installing — sent on `installed` in the second case, after a
/// `statechange` wait — and a `controllerchange` arriving with no reload of
/// ours in flight is answered by one reload, so a version swap is never
/// better than one reload away, whatever path it came in on.
///
/// Best effort like everything around it: a failure is a console line, and an
/// app that stays on this version still runs.
async fn nudge_worker_update(registration: &web_sys::ServiceWorkerRegistration) {
    let Some(window) = web_sys::window() else {
        return;
    };

    // The reload that follows a swap. The flag keeps it one-per-swap, so a
    // late `updatefound` from the same registration cannot reload again
    // underneath it.
    watch_for_controller_change(&window);

    // A worker installed and waiting for pages like this one to close. This
    // is the state an update is left in when a visit ended before its
    // handover: nothing new will arrive until the served bytes change again,
    // so this is the one chance to ask it in.
    if let Some(worker) = registration.waiting() {
        if worker.state() == web_sys::ServiceWorkerState::Installed {
            ask_worker_in(&worker);
            return;
        }
    }

    // A worker installing right now — this visit's own update check found
    // one — or one that arrives later while the page stands.
    let installing = match registration.installing() {
        Some(worker) => Some(worker),
        None => update_found(registration).await,
    };
    let Some(worker) = installing else {
        return;
    };
    if wait_for_state_change(&worker).await.is_err() {
        warn("could not wait for the worker to install", &JsValue::NULL);
        return;
    }
    if worker.state() == web_sys::ServiceWorkerState::Installed {
        ask_worker_in(&worker);
    }
}

/// Send the message the worker answers with `skipWaiting`.
fn ask_worker_in(worker: &web_sys::ServiceWorker) {
    worker.post_message(&JsValue::from_str("SKIP_WAITING")).ok();
}

/// Reload once when the worker in charge changes.
///
/// The guard is the in-flight flag: an ask sent twice would fire the event
/// once, and a second registration's late update could fire it again under a
/// page already reloading. One reload per swap, and never two in a row, is
/// what keeps this from becoming the loop the naive version of this code is:
/// a page that reloads unasked on `installed` never lets the waiting worker
/// activate, because a reloading page still counts as a client.
fn watch_for_controller_change(window: &web_sys::Window) {
    thread_local! {
        static WATCHING: Cell<bool> = const { Cell::new(false) };
    }
    if WATCHING.with(Cell::get) {
        return;
    }
    WATCHING.with(|watching| watching.set(true));

    let container = window.navigator().service_worker();
    let handler = Closure::<dyn FnMut(JsValue)>::new(move |_event: JsValue| {
        if let Some(window) = web_sys::window() {
            window.location().reload().ok();
        }
    });
    let _ = container
        .add_event_listener_with_callback("controllerchange", handler.as_ref().unchecked_ref());
    // Never dropped: exactly one listener for the page's lifetime, and the
    // guard above makes re-registering a no-op.
    handler.forget();
}

/// The worker this registration announces through `updatefound`, if one
/// arrives while the page stands.
///
/// The event carries the registration, not the worker; the worker is its
/// `installing` one. `None` resolves the wait empty if the registration is
/// ever torn down between the two states this cares about.
async fn update_found(
    registration: &web_sys::ServiceWorkerRegistration,
) -> Option<web_sys::ServiceWorker> {
    let event = JsFuture::from(js_sys::Promise::new(&mut |resolve, _reject| {
        let resolved = Closure::<dyn FnMut(JsValue)>::new(move |value: JsValue| {
            resolve.call1(&JsValue::NULL, &value).ok();
        });
        let _ = registration
            .add_event_listener_with_callback("updatefound", resolved.as_ref().unchecked_ref());
        // Never dropped: the promise settles at most once, and the
        // registration outlives the page anyway.
        resolved.forget();
    }))
    .await
    .ok()?;
    let event = event.dyn_into::<web_sys::Event>().ok()?;
    let announced = event
        .target()?
        .dyn_into::<web_sys::ServiceWorkerRegistration>()
        .ok()?;
    announced.installing()
}

/// Resolve at the worker's next state change, whichever it is.
async fn wait_for_state_change(worker: &web_sys::ServiceWorker) -> Result<(), JsValue> {
    JsFuture::from(js_sys::Promise::new(&mut |resolve, _reject| {
        let resolved = Closure::<dyn FnMut(JsValue)>::new(move |value: JsValue| {
            resolve.call1(&JsValue::NULL, &value).ok();
        });
        let _ = worker
            .add_event_listener_with_callback("statechange", resolved.as_ref().unchecked_ref());
        // Never dropped, like the one above: at most one state change is
        // waited for here, and the worker outlives the page.
        resolved.forget();
    }))
    .await
    .map(|_| ())
}

/// Hand this app's own URLs back to the current worker.
///
/// A registration is kept by the browser, not the tab, so it outlives the page
/// that made it and keeps answering for its scope until something explicitly
/// unregisters it. That is how a page on this origin comes to be served by a
/// worker installed for a *different* app, long after that app was closed — and
/// a reload, a newer version or a newer worker installing itself cannot fix
/// it, because the newer worker only takes control where its own scope reaches.
///
/// So: unregister any registration whose scope covers this app's directory but
/// is not this app's directory. Failures are ignored — this is best-effort
/// cleanup of state this app did not create, and a browser that refuses leaves
/// the user no worse off.
async fn release_stale_registrations(window: &Window) {
    let container = window.navigator().service_worker();
    let Ok(registrations) = JsFuture::from(container.get_registrations()).await else {
        return;
    };
    let Ok(array) = registrations.dyn_into::<js_sys::Array>() else {
        return;
    };

    // This app's own directory, as an absolute URL with a trailing slash. The
    // app is served from a subdirectory and every URL of ours is inside it.
    let Ok(home) = window.location().href() else {
        return;
    };
    let Ok(ours) = web_sys::Url::new_with_base(&home, "./") else {
        return;
    };
    let ours = ours.href();

    for entry in array.iter() {
        let Ok(registration) = entry.dyn_into::<web_sys::ServiceWorkerRegistration>() else {
            continue;
        };
        let scope = registration.scope();
        // Leave alone any scope that is this app's own, or narrower: a sibling
        // app mounted inside this directory is legitimate and separate, and
        // nothing there can intercept us. One test, because the two cases are
        // the same one: `ours` begins with `scope`.
        if ours.starts_with(&scope) {
            continue;
        }
        // What is left is a scope that is a *strict* prefix of ours: a worker that
        // would be consulted for this app's URLs while registered for more of
        // the origin. Only our own worker qualifies — a different app's worker
        // lives in a different directory, so unregistering it would break the
        // app it belongs to.
        let script = registration
            .active()
            .map(|worker| worker.script_url())
            .unwrap_or_default();
        if script_belongs_to_app(&script, &ours) {
            match registration.unregister() {
                Ok(promise) => {
                    let _ = JsFuture::from(promise).await;
                }
                Err(error) => warn("stale service worker could not be released", &error),
            }
        }
    }
}

/// Whether a worker script at `script` is this app's own worker, registered for
/// more of the origin than this app's directory.
///
/// A wider scope means the script sits at the root of this app's own directory
/// rather than below it: a sibling app's worker is in a sibling directory and
/// does not match. `strip_suffix`, not `trim_end_matches` — the latter strips a
/// *set of characters*, so it would eat a directory named `...e-worker.js` and
/// call it ours.
fn script_belongs_to_app(script: &str, ours: &str) -> bool {
    match web_sys::Url::new(script) {
        Ok(url) => url.href().strip_suffix("service-worker.js") == Some(ours),
        Err(_) => false,
    }
}

/// A new `<span>` with one class, typed for styling.
fn span(document: &Document, class: &str) -> Option<HtmlElement> {
    let node = document.create_element("span").ok()?;
    node.set_class_name(class);
    node.dyn_into().ok()
}

/// A new element of any tag with one class.
fn node(document: &Document, tag: &str, class: &str) -> Option<Element> {
    let node = document.create_element(tag).ok()?;
    node.set_class_name(class);
    Some(node)
}

/// A document element, or `None`.
///
/// Never panics: the shell is a separate file that someone will edit, and a
/// missing id should degrade the app, not abort it.
fn element(document: &Document, id: &str) -> Option<Element> {
    document.get_element_by_id(id)
}

/// The typed form of [`element`], so button methods are available without a cast.
fn button(document: &Document, id: &str) -> Option<HtmlButtonElement> {
    element(document, id)?.dyn_into().ok()
}

/// The typed form of [`element`] for the settings selects.
fn select_element(document: &Document, id: &str) -> Option<web_sys::HtmlSelectElement> {
    element(document, id)?.dyn_into().ok()
}

/// The typed form of [`element`] for the settings checkbox.
fn input_element(document: &Document, id: &str) -> Option<web_sys::HtmlInputElement> {
    element(document, id)?.dyn_into().ok()
}

/// Replace an element's text.
fn set_text(document: &Document, id: &str, text: &str) {
    if let Some(element) = element(document, id) {
        element.set_text_content(Some(text));
    }
}

/// Paint an element by id, or clear it with an empty string.
fn paint(document: &Document, id: &str, value: &str) {
    let typed: Option<HtmlElement> = element(document, id).and_then(|node| node.dyn_into().ok());
    if let Some(element) = typed {
        set_style(&element, "background", value);
    }
}

/// Replace a button's text, keeping its typed handle.
fn set_text_element(button: &HtmlButtonElement, text: &str) {
    button.set_text_content(Some(text));
}

/// Paint an element's background, or clear it with an empty string.
fn set_style(element: &HtmlElement, property: &str, value: &str) {
    let _ = element.style().set_property(property, value);
}

/// Set one attribute, ignoring a browser that refuses.
fn set_attribute(element: &Element, name: &str, value: &str) {
    let _ = element.set_attribute(name, value);
}

/// Empty an element: every child gone, the element itself kept.
fn clear(host: &Element) {
    while let Some(child) = host.last_child() {
        let _ = host.remove_child(&child);
    }
}

/// Report something recoverable. Never a panic, never silent.
fn warn(context: &str, error: &JsValue) {
    web_sys::console::warn_2(&JsValue::from_str(context), error);
}
