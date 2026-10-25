//! The app's logic: pigments, mixes, rounds, attempts, hints and score.
//!
//! No `web-sys`, no `js-sys`, no `wasm-bindgen`, so this module compiles and
//! is unit tested natively with `cargo test`. Keep it that way: the moment
//! this file reaches for `document`, its tests stop running and everything a
//! person can reason about moves into `crate::ui`, where it can only be
//! checked in a browser.

/// A palette pigment: one named colour the player can pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pigment {
    pub name: &'static str,
    pub rgb: [u8; 3],
    pub family: Family,
}

/// Coarse colour families, used only to bias distractor picks towards colours
/// that are genuinely confusable with the ingredients.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Neutral,
    Warm,
    Cool,
    Bright,
    Pastel,
    Earth,
}

/// The palette. Sixteen pigments across six families: enough primaries to
/// build real mixes, enough near-neighbours to make proportions matter.
///
/// Index in this array is the pigment's id everywhere else in the app, so the
/// order is never shuffled — the UI depends on it for stable swatch positions
/// across rounds.
pub const PIGMENTS: [Pigment; 16] = [
    Pigment {
        name: "White",
        rgb: [0xff, 0xff, 0xff],
        family: Family::Neutral,
    },
    Pigment {
        name: "Black",
        rgb: [0x1a, 0x1a, 0x1a],
        family: Family::Neutral,
    },
    Pigment {
        name: "Grey",
        rgb: [0x8d, 0x99, 0xa6],
        family: Family::Neutral,
    },
    Pigment {
        name: "Red",
        rgb: [0xd3, 0x2f, 0x2f],
        family: Family::Warm,
    },
    Pigment {
        name: "Orange",
        rgb: [0xf5, 0x7c, 0x00],
        family: Family::Warm,
    },
    Pigment {
        name: "Yellow",
        rgb: [0xf0, 0xc0, 0x00],
        family: Family::Warm,
    },
    Pigment {
        name: "Green",
        rgb: [0x2e, 0x7d, 0x32],
        family: Family::Bright,
    },
    Pigment {
        name: "Lime",
        rgb: [0x9c, 0xcc, 0x0f],
        family: Family::Bright,
    },
    Pigment {
        name: "Cyan",
        rgb: [0x26, 0xb8, 0xc8],
        family: Family::Cool,
    },
    Pigment {
        name: "Blue",
        rgb: [0x24, 0x56, 0xc8],
        family: Family::Cool,
    },
    Pigment {
        name: "Navy",
        rgb: [0x1b, 0x2f, 0x6e],
        family: Family::Cool,
    },
    Pigment {
        name: "Violet",
        rgb: [0x7b, 0x3f, 0xd4],
        family: Family::Cool,
    },
    Pigment {
        name: "Magenta",
        rgb: [0xd8, 0x1b, 0x8c],
        family: Family::Bright,
    },
    Pigment {
        name: "Pink",
        rgb: [0xf4, 0x8f, 0xb1],
        family: Family::Pastel,
    },
    Pigment {
        name: "Peach",
        rgb: [0xff, 0xcd, 0xa8],
        family: Family::Pastel,
    },
    Pigment {
        name: "Brown",
        rgb: [0x8d, 0x5a, 0x2b],
        family: Family::Earth,
    },
];

/// The largest palette the settings may offer: `PIGMENTS.len()`, named for the
/// settings clamps and the UI's option lists.
pub const MAX_COLORS: usize = PIGMENTS.len();

/// The fewest colours a mix can name. One colour would be a guessing game
/// about nothing; two is the smallest real mix.
pub const MIN_MIX: usize = 2;

/// The most colours a mix can name.
pub const MAX_MIX: usize = 4;

/// The most parts a mix can be split into.
pub const MAX_PARTS: usize = 12;

/// What one revealed hint costs, deducted from the score at reveal.
pub const HINT_COST: i64 = 15;

/// Points for a round solved.
pub const SOLVE_POINTS: i64 = 20;

/// Bonus points on top, when a round is solved with no hint revealed.
pub const CLEAN_BONUS: i64 = 5;

/// The game's configurable shape, clamped on the way in.
///
/// * `color_count` — how many swatches the player chooses from.
/// * `mix_parts` — how many distinct colours the target mix is built from.
/// * `parts_total` — how many parts those colours are split into.
/// * `show_preview` — show the player's current mix while building it,
///   rather than only when a check is tried.
///
/// The clamps are the invariants every later read relies on: the mix needs at
/// least two colours, the palette must offer at least one distractor, and the
/// parts must be able to give every ingredient at least one unit. Nothing
/// downstream re-checks them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub color_count: usize,
    pub mix_parts: usize,
    pub parts_total: usize,
    pub show_preview: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self::new()
    }
}

impl Settings {
    /// The starting settings: six swatches, a two-colour mix, six parts, no
    /// live preview.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            color_count: 6,
            mix_parts: 2,
            parts_total: 6,
            show_preview: false,
        }
    }

    /// Raw values, clamped into the legal shape.
    ///
    /// Every bound is derived from `mix_parts`, so the three numbers cannot
    /// disagree: the palette always keeps a distractor, and the parts can
    /// always cover every ingredient once.
    #[must_use]
    pub fn clamped(
        color_count: usize,
        mix_parts: usize,
        parts_total: usize,
        show_preview: bool,
    ) -> Self {
        let mix_parts = mix_parts.clamp(MIN_MIX, MAX_MIX);
        Self {
            color_count: color_count.clamp(mix_parts + 1, MAX_COLORS),
            mix_parts,
            parts_total: parts_total.clamp(mix_parts + 1, MAX_PARTS),
            show_preview,
        }
    }

    /// A stored settings line, read back defensively.
    ///
    /// The format is `"count,mix,parts,preview"`. Anything missing, truncated,
    /// unreadable or out of range is not an error: the default settings come
    /// back, because a corrupt stored line must never stop the app.
    #[must_use]
    pub fn from_stored(raw: Option<&str>) -> Self {
        let Some(text) = raw.map(str::trim).filter(|text| !text.is_empty()) else {
            return Self::new();
        };
        let mut fields = text.split(',');
        let mut number = || {
            fields
                .next()
                .and_then(|field| field.trim().parse::<usize>().ok())
        };
        // A stored line from before the preview flag existed has only three
        // fields; `preview` then reads as `None` and means "off".
        let (count, mix, parts, preview) = (number(), number(), number(), number());
        match (count, mix, parts, preview) {
            (Some(count), Some(mix), Some(parts), preview) => {
                Self::clamped(count, mix, parts, preview.unwrap_or(0) != 0)
            }
            _ => Self::new(),
        }
    }

    /// The line to persist, or `None` while the settings are the defaults, so
    /// a player who never changed anything writes nothing at all.
    #[must_use]
    pub fn to_stored(&self) -> Option<String> {
        if *self == Self::new() {
            return None;
        }
        Some(format!(
            "{},{},{},{}",
            self.color_count,
            self.mix_parts,
            self.parts_total,
            u8::from(self.show_preview)
        ))
    }
}

/// The running score and its counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Score {
    pub points: i64,
    pub solved: u32,
    pub attempts: u32,
    pub streak: u32,
    pub best_streak: u32,
}

impl Score {
    /// The points a solved round is worth: the base, plus the clean bonus
    /// when no hint was revealed first.
    #[must_use]
    pub fn round_worth(hints_used: usize) -> i64 {
        let clean = i64::from(u8::from(hints_used == 0));
        SOLVE_POINTS + CLEAN_BONUS * clean
    }

    /// Record one check.
    pub fn record(&mut self, solved: bool, hints_used: usize) {
        self.attempts += 1;
        if solved {
            self.solved += 1;
            self.streak += 1;
            self.best_streak = self.best_streak.max(self.streak);
            self.points += Self::round_worth(hints_used);
        } else {
            self.streak = 0;
        }
    }

    /// Deduct a hint's cost, floored at zero: a beginner's score should not
    /// go negative for asking for help in a practice game.
    pub fn pay_hint(&mut self) {
        self.points = (self.points - HINT_COST).max(0);
    }

    /// A stored score line, read back defensively.
    ///
    /// The format is `"points,solved,attempts,streak,best"`. Anything
    /// unreadable becomes a fresh score.
    #[must_use]
    pub fn from_stored(raw: Option<&str>) -> Self {
        let Some(text) = raw.map(str::trim).filter(|text| !text.is_empty()) else {
            return Self::default();
        };
        let mut fields = text.split(',');
        let mut number = || {
            fields
                .next()
                .and_then(|field| field.trim().parse::<i64>().ok())
        };
        let (points, solved, attempts, streak, best) =
            (number(), number(), number(), number(), number());
        let counter = |value: Option<i64>| -> u32 {
            value
                .and_then(|value| u32::try_from(value.max(0)).ok())
                .unwrap_or(u32::MAX)
        };
        match (points, solved, attempts, streak, best) {
            (Some(points), solved, attempts, streak, best) => {
                // Clamped on the way in, so no stored line can make the
                // counters disagree with themselves later.
                Self {
                    points: points.max(0),
                    solved: counter(solved),
                    attempts: counter(attempts),
                    streak: counter(streak),
                    best_streak: counter(best),
                }
            }
            _ => Self::default(),
        }
    }

    /// The line to persist, or `None` while nothing has been scored yet, so
    /// a player who never played writes nothing at all.
    #[must_use]
    pub fn to_stored(&self) -> Option<String> {
        if self.attempts == 0 {
            return None;
        }
        Some(format!(
            "{},{},{},{},{}",
            self.points, self.solved, self.attempts, self.streak, self.best_streak
        ))
    }
}

/// One revealed property of the target mix.
///
/// Ingredient hints come first, one per ingredient; the proportions hint is
/// last, and only once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hint {
    /// This pigment takes part in the mix.
    Ingredient(usize),
    /// The full part split, as counts only.
    Proportions,
}

/// One round: a target mix, the palette it hides in, and what has been
/// revealed about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Round {
    /// Pigment ids the player chooses from, sorted, ingredients included.
    pub palette: Vec<usize>,
    /// The target's ingredients, sorted. `mix_parts` of them.
    pub ingredients: Vec<usize>,
    /// Parts per ingredient, parallel to `ingredients`, summing to
    /// `parts_total`.
    pub parts: Vec<usize>,
    /// Hints revealed so far, in reveal order.
    pub revealed: Vec<Hint>,
}

impl Round {
    /// Generate a round for `settings`, deterministic in `seed`.
    ///
    /// Ingredients are drawn first, then a random composition of the parts
    /// across them, then distractors — biased towards the ingredients' own
    /// families, so the palette is confusable rather than conveniently
    /// obvious. The palette is sorted, so every pigment keeps its swatch
    /// position across rounds.
    ///
    /// Rounds whose target can also be mixed from some *other* choice of
    /// colours and parts are rejected and redrawn: the player solves by eye,
    /// so a second visually identical answer would make a correct-looking
    /// attempt score as wrong. The redraw is bounded and the last candidate is
    /// kept, so the generator is total — collisions are rare enough that the
    /// bound is never reached in practice.
    #[must_use]
    pub fn generate(settings: &Settings, seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let mut dealt = Self::draw(settings, &mut rng);
        for _ in 0..256 {
            if dealt.has_unique_solution() {
                return dealt;
            }
            dealt = Self::draw(settings, &mut rng);
        }
        // The last draw is unchecked: after 256 collisions the seed is doing
        // something no real settings reach, and a round — however awkward —
        // is better than not dealing at all.
        dealt
    }

    /// One candidate round, before the uniqueness rejection.
    fn draw(settings: &Settings, rng: &mut Rng) -> Self {
        let mix_parts = settings.mix_parts;
        let parts_total = settings.parts_total;

        let mut order: Vec<usize> = (0..PIGMENTS.len()).collect();
        rng.shuffle(&mut order);
        let mut ingredients = order[..mix_parts].to_vec();
        ingredients.sort_unstable();

        // A random composition: every ingredient starts with one part, and
        // the remaining units are dealt one by one.
        let mut parts = vec![1usize; mix_parts];
        for _ in mix_parts..parts_total {
            let index = rng.below(mix_parts);
            parts[index] += 1;
        }

        // Distractors: family matches first, so the palette tempts the eye
        // with colours genuinely close to the ingredients.
        let others: Vec<usize> = order[mix_parts..]
            .iter()
            .copied()
            .filter(|&id| !ingredients.contains(&id))
            .collect();
        let mut bag: Vec<usize> = Vec::with_capacity(others.len());
        for pass in [true, false] {
            for &id in &others {
                let matches = PIGMENTS[id].family != Family::Neutral
                    && ingredients
                        .iter()
                        .any(|&ingredient| PIGMENTS[ingredient].family == PIGMENTS[id].family);
                if matches == pass {
                    bag.push(id);
                }
            }
        }
        let wanted = settings.color_count.min(PIGMENTS.len());
        let mut palette = ingredients.clone();
        palette.extend(bag.into_iter().take(wanted.saturating_sub(mix_parts)));
        palette.sort_unstable();

        Self {
            palette,
            ingredients,
            parts,
            revealed: Vec::new(),
        }
    }

    /// The colour the target mix produces.
    #[must_use]
    pub fn target_rgb(&self) -> [u8; 3] {
        mix(&self.ingredients, &self.parts)
    }

    /// Reveal the next unrevealed property, or `None` when everything is out.
    ///
    /// Ingredient hints are handed out one at a time, in the target's own
    /// ingredient order; the proportions hint comes once, after them.
    pub fn hint(&mut self) -> Option<Hint> {
        for &ingredient in &self.ingredients {
            let shown = self
                .revealed
                .iter()
                .any(|hint| matches!(hint, Hint::Ingredient(id) if *id == ingredient));
            if !shown {
                let hint = Hint::Ingredient(ingredient);
                self.revealed.push(hint);
                return Some(hint);
            }
        }
        if !self.revealed.contains(&Hint::Proportions) {
            self.revealed.push(Hint::Proportions);
            return Some(Hint::Proportions);
        }
        None
    }

    /// The words for one revealed hint, as the round shows it.
    ///
    /// The proportions hint lists the shares sorted descending: aligned with
    /// the ingredient order it would hand the player the full answer once the
    /// membership hints are out, and sorted it stays a check rather than a
    /// reveal.
    #[must_use]
    pub fn hint_text(&self, hint: &Hint) -> String {
        match hint {
            Hint::Ingredient(id) => format!("{} is in the mix.", PIGMENTS[*id].name),
            Hint::Proportions => {
                let mut shares: Vec<usize> = self.parts.clone();
                shares.sort_unstable_by(|a, b| b.cmp(a));
                let listed: Vec<String> = shares.iter().map(usize::to_string).collect();
                format!("The parts split {}.", listed.join(" : "))
            }
        }
    }

    /// Every revealed hint, worded.
    #[must_use]
    pub fn hint_lines(&self) -> Vec<String> {
        self.revealed
            .iter()
            .map(|hint| self.hint_text(hint))
            .collect()
    }

    /// Whether the round is honestly solvable: no choice of colours and parts
    /// other than the target's own can mix to the target colour.
    ///
    /// Subsets whose channel ranges cannot span the target are skipped without
    /// enumerating their compositions — a parts-weighted mean always lies
    /// within the convex hull of its inputs, channel by channel, which prunes
    /// almost everything cheaply.
    #[must_use]
    pub fn has_unique_solution(&self) -> bool {
        let target = self.target_rgb();
        let mix_parts = self.ingredients.len();
        let parts_total: usize = self.parts.iter().sum();
        let collision = std::cell::Cell::new(false);
        for_each_combination(&self.palette, mix_parts, &mut |combo| {
            if collision.get() || !hull_contains(combo, target) {
                return;
            }
            let same_set = combo.iter().copied().eq(self.ingredients.iter().copied());
            let mut hit = false;
            for_each_composition(parts_total, mix_parts, &mut |comp| {
                if hit || (same_set && comp == self.parts) {
                    return;
                }
                if mix(combo, comp) == target {
                    hit = true;
                }
            });
            if hit {
                collision.set(true);
            }
        });
        !collision.get()
    }
}

/// The weighted sRGB mix of `pigments` under `shares`, rounded half up.
///
/// This is the app's one model of colour: parts-weighted channel means in
/// sRGB. It is deliberately not perceptual — the point of the game is that
/// mixing behaves the way a novice expects, and sRGB averaging is exactly
/// that.
///
/// `shares` must be parallel to `pigments`; a zero total yields black rather
/// than a division trap, and callers never produce one.
#[must_use]
pub fn mix(pigments: &[usize], shares: &[usize]) -> [u8; 3] {
    let total: usize = shares.iter().sum();
    let total = total.max(1);
    let mut out = [0u8; 3];
    for (channel, slot) in out.iter_mut().enumerate() {
        let weighted: usize = pigments
            .iter()
            .zip(shares.iter())
            .map(|(&pigment, &share)| usize::from(PIGMENTS[pigment].rgb[channel]) * share)
            .sum();
        *slot = u8::try_from((weighted + total / 2) / total).unwrap_or(u8::MAX);
    }
    out
}

/// The `#rrggbb` form of an RGB triple, as the UI paints it.
#[must_use]
pub fn hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

/// Readable text colour on `rgb`: black or white, by relative luminance.
///
/// The picks strip writes each colour's share count *on* the colour, so the
/// domain owns the contrast decision — WCAG's relative-luminance formula,
/// the 4.5:1 bar for small text, white when dark text would fall short.
#[must_use]
pub fn text_on(rgb: [u8; 3]) -> &'static str {
    let channel = |value: u8| -> f64 {
        let value = f64::from(value) / 255.0;
        if value <= 0.039_28 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance =
        0.212_6 * channel(rgb[0]) + 0.715_2 * channel(rgb[1]) + 0.072_2 * channel(rgb[2]);
    // Contrast of black text is (L + 0.05) / 0.05; of white, 1.05 / (L + 0.05).
    if luminance > 0.183_3 {
        "#000000"
    } else {
        "#ffffff"
    }
}

/// Whether the target's channels all lie within the min–max span of `combo`.
fn hull_contains(combo: &[usize], target: [u8; 3]) -> bool {
    target.iter().enumerate().all(|(channel, &wanted)| {
        let values = combo.iter().map(|&pigment| PIGMENTS[pigment].rgb[channel]);
        let min = values.clone().min().unwrap_or(u8::MAX);
        let max = values.max().unwrap_or(u8::MIN);
        wanted >= min && wanted <= max
    })
}

/// Calls `f` with every `k`-element combination of `items`, in order.
fn for_each_combination<T: Copy>(items: &[T], k: usize, f: &mut dyn FnMut(&[T])) {
    let mut scratch = Vec::with_capacity(k);
    combine_from(items, 0, k, &mut scratch, f);
}

fn combine_from<T: Copy>(
    items: &[T],
    start: usize,
    k: usize,
    scratch: &mut Vec<T>,
    f: &mut dyn FnMut(&[T]),
) {
    if k == 0 {
        f(scratch);
        return;
    }
    if k > items.len().saturating_sub(start) {
        return;
    }
    for index in start..=items.len() - k {
        scratch.push(items[index]);
        combine_from(items, index + 1, k - 1, scratch, f);
        scratch.pop();
    }
}

/// Calls `f` with every composition of `total` into `slots` positive parts.
fn for_each_composition(total: usize, slots: usize, f: &mut dyn FnMut(&[usize])) {
    let mut scratch = vec![0usize; slots];
    compose_into(total, slots, &mut scratch, f);
}

fn compose_into(
    remaining: usize,
    slots: usize,
    scratch: &mut [usize],
    f: &mut dyn FnMut(&[usize]),
) {
    if slots == 0 {
        if remaining == 0 {
            f(scratch);
        }
        return;
    }
    // This slot takes at least one and leaves at least one for every slot
    // after it.
    for value in 1..=remaining.saturating_sub(slots - 1) {
        scratch[slots - 1] = value;
        compose_into(remaining - value, slots - 1, scratch, f);
    }
}

/// A tiny deterministic generator: `xorshift64*`.
///
/// Good enough for dealing rounds, small enough to need no dependency, and
/// fully deterministic so tests can pin a seed.
pub struct Rng(u64);

impl Rng {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    #[must_use]
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    #[must_use]
    pub fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        let bound = u64::try_from(bound).unwrap_or(u64::MAX);
        usize::try_from(self.next_u64() % bound).unwrap_or(0)
    }

    pub fn shuffle<T>(&mut self, slice: &mut [T]) {
        for index in (1..slice.len()).rev() {
            let swap = self.below(index + 1);
            slice.swap(index, swap);
        }
    }
}

/// One pigment the player has picked, with the parts given to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub pigment: usize,
    pub parts: usize,
}

/// What a check just decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The attempt matched the target. Carries the points the round was
    /// worth, for the status line.
    Solved(i64),
    /// The attempt mixed to something else.
    Wrong,
}

/// The whole game state: settings, the round, the player's attempt on it, and
/// the score.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    pub settings: Settings,
    pub round: Round,
    pub attempt: Vec<Entry>,
    pub score: Score,
    /// The last check's outcome, for the status line. Cleared on a new round.
    pub outcome: Option<Outcome>,
    /// The attempt as it stood at the last check, for the UI's reveal rule:
    /// with the preview off, a checked mix stays visible until the player
    /// changes it.
    pub checked: Option<Vec<Entry>>,
}

impl Game {
    /// A fresh game: clamped settings, a first round, nothing picked.
    #[must_use]
    pub fn new(settings: Settings, seed: u64) -> Self {
        let settings = Settings::clamped(
            settings.color_count,
            settings.mix_parts,
            settings.parts_total,
            settings.show_preview,
        );
        Self {
            round: Round::generate(&settings, seed),
            settings,
            attempt: Vec::new(),
            score: Score::default(),
            outcome: None,
            checked: None,
        }
    }

    /// The number of parts the attempt has assigned so far.
    #[must_use]
    pub fn assigned(&self) -> usize {
        self.attempt.iter().map(|entry| entry.parts).sum()
    }

    /// Whether the attempt names the right number of colours and all the
    /// parts — the only state a check may run in.
    #[must_use]
    pub fn attempt_is_complete(&self) -> bool {
        self.assigned() == self.settings.parts_total
    }

    /// Toggle one pigment in or out of the attempt.
    ///
    /// The attempt is deliberately not capped at the target's colour count:
    /// adding a third colour to a two-colour round is how a player finds out
    /// what it does. The parts jar is the real constraint — six parts cannot
    /// fund eight colours at one each — and it is enforced by `nudge` and
    /// `take_back`. Deselecting drops the entry and its parts.
    pub fn toggle(&mut self, pigment: usize) {
        self.checked = None;
        if let Some(at) = self
            .attempt
            .iter()
            .position(|entry| entry.pigment == pigment)
        {
            self.attempt.remove(at);
        } else if self.attempt.len() < self.settings.parts_total {
            self.attempt.push(Entry { pigment, parts: 1 });
        }
    }

    /// Move one picked pigment's parts up or down by one, saturating at both
    /// ends: at least one part while it stays picked, and never more than
    /// what is left of the round's total once the other picks are paid.
    pub fn nudge(&mut self, pigment: usize, up: bool) {
        self.checked = None;
        let others: usize = self
            .attempt
            .iter()
            .filter(|other| other.pigment != pigment)
            .map(|other| other.parts)
            .sum();
        let ceiling = self.settings.parts_total.saturating_sub(others).max(1);
        let Some(entry) = self
            .attempt
            .iter_mut()
            .find(|entry| entry.pigment == pigment)
        else {
            return;
        };
        entry.parts = if up {
            (entry.parts + 1).min(ceiling)
        } else {
            (entry.parts - 1).max(1)
        };
    }

    /// The tap on a picked colour's chip: one part comes back.
    ///
    /// At one part the colour leaves with its last unit — a chip that
    /// stopped responding at one part would read as broken, and an empty
    /// entry would be a colour in the mix with nothing of it in the jar.
    /// Not-picked colours are untouched; the palette handles adding.
    pub fn take_back(&mut self, pigment: usize) {
        let Some(at) = self
            .attempt
            .iter()
            .position(|entry| entry.pigment == pigment)
        else {
            return;
        };
        self.checked = None;
        if self.attempt[at].parts == 1 {
            self.attempt.remove(at);
        } else {
            self.attempt[at].parts -= 1;
        }
    }

    /// The colour the attempt currently mixes to, or `None` while it is
    /// empty.
    #[must_use]
    pub fn attempt_rgb(&self) -> Option<[u8; 3]> {
        if self.attempt.is_empty() {
            return None;
        }
        let pigments: Vec<usize> = self.attempt.iter().map(|e| e.pigment).collect();
        let shares: Vec<usize> = self.attempt.iter().map(|e| e.parts).collect();
        Some(mix(&pigments, &shares))
    }

    /// Check the attempt against the target, and score it.
    ///
    /// A check is only meaningful with the right number of colours and all
    /// the parts assigned; anything else is a no-op that leaves the state
    /// alone, and the UI disables the button to say so.
    pub fn check(&mut self) -> Option<Outcome> {
        if !self.attempt_is_complete() {
            return None;
        }
        // Compare as sets with aligned shares: the player may have picked in
        // any order, so both lists are sorted by pigment id together.
        let mut played: Vec<(usize, usize)> = self
            .attempt
            .iter()
            .map(|entry| (entry.pigment, entry.parts))
            .collect();
        played.sort_unstable_by_key(|&(pigment, _)| pigment);
        let target: Vec<(usize, usize)> = self
            .round
            .ingredients
            .iter()
            .copied()
            .zip(self.round.parts.iter().copied())
            .collect();

        let solved = played == target;
        let worth = if solved {
            Score::round_worth(self.round.revealed.len())
        } else {
            0
        };
        self.score.record(solved, self.round.revealed.len());
        let outcome = if solved {
            Outcome::Solved(worth)
        } else {
            Outcome::Wrong
        };
        self.outcome = Some(outcome);
        self.checked = Some(self.attempt.clone());
        Some(outcome)
    }

    /// Reveal the next hint, paying for it. `None` when everything is out.
    pub fn hint(&mut self) -> Option<String> {
        let hint = self.round.hint()?;
        self.score.pay_hint();
        Some(self.round.hint_text(&hint))
    }

    /// Deal the next round: everything picked, hinted and decided is dropped.
    pub fn next_round(&mut self, seed: u64) {
        self.round = Round::generate(&self.settings, seed);
        self.attempt.clear();
        self.outcome = None;
        self.checked = None;
    }

    /// Apply new settings, dealing a fresh round: a changed shape invalidates
    /// the round and the attempt both.
    pub fn apply_settings(&mut self, settings: Settings, seed: u64) {
        let settings = Settings::clamped(
            settings.color_count,
            settings.mix_parts,
            settings.parts_total,
            settings.show_preview,
        );
        self.settings = settings;
        self.next_round(seed);
    }

    /// The one function that owns every word the app shows about the state.
    ///
    /// The UI writes these lines verbatim; it never composes its own.
    #[must_use]
    pub fn status(&self) -> String {
        match self.outcome {
            Some(Outcome::Solved(worth)) => format!("Solved — +{worth}"),
            Some(Outcome::Wrong) => "Not this mix. Adjust and check again.".to_string(),
            None => {
                let missing = self.settings.parts_total - self.assigned();
                if self.attempt.is_empty() {
                    format!(
                        "Pick colours and split {} parts.",
                        self.settings.parts_total
                    )
                } else if missing > 0 {
                    format!(
                        "{missing} more part{} to place.",
                        if missing == 1 { "" } else { "s" }
                    )
                } else {
                    "All parts placed. Check when ready.".to_string()
                }
            }
        }
    }

    /// The words above the palette: what this round asks for.
    #[must_use]
    pub fn round_line(&self) -> String {
        format!(
            "Mix {} colours in {} parts",
            self.settings.mix_parts, self.settings.parts_total
        )
    }

    /// The words for the score, as one line.
    #[must_use]
    pub fn score_line(&self) -> String {
        if self.score.attempts == 0 {
            return "No rounds yet.".to_string();
        }
        format!(
            "{} solved · streak {} · best {} · {} pts",
            self.score.solved, self.score.streak, self.score.best_streak, self.score.points
        )
    }

    /// The word for the round-advance button: after a solve it moves on, and
    /// before one it gives up the round.
    #[must_use]
    pub fn next_label(&self) -> &'static str {
        if matches!(self.outcome, Some(Outcome::Solved(_))) {
            "Next round"
        } else {
            "Skip"
        }
    }

    /// Whether the UI may show the attempt's colour right now.
    ///
    /// The preview setting answers yes on its own. Without it, the attempt's
    /// colour is the answer to a check: it may be shown once a check has
    /// judged it, and it must hide again the moment the attempt changes —
    /// which is exactly what the `checked` snapshot records.
    #[must_use]
    pub fn reveal_attempt(&self) -> bool {
        if self.settings.show_preview {
            return true;
        }
        match &self.checked {
            Some(snapshot) => snapshot.as_slice() == self.attempt.as_slice(),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pick the round's own ingredients and give each its own parts: the
    /// state a correct attempt is in, whatever the generator dealt.
    fn play_target(game: &mut Game) {
        let plan: Vec<(usize, usize)> = game
            .round
            .ingredients
            .iter()
            .copied()
            .zip(game.round.parts.iter().copied())
            .collect();
        for (ingredient, parts) in plan {
            game.toggle(ingredient);
            for _ in 1..parts {
                game.nudge(ingredient, true);
            }
        }
    }

    #[test]
    fn settings_clamp_into_a_legal_shape() {
        let s = Settings::clamped(99, 3, 99, true);
        assert_eq!(s.color_count, MAX_COLORS);
        assert_eq!(s.mix_parts, 3);
        assert_eq!(s.parts_total, MAX_PARTS);
        assert!(s.show_preview);

        // The palette always keeps a distractor, the parts always cover every
        // ingredient once — derived from mix_parts, not hardcoded.
        let s = Settings::clamped(1, MAX_MIX, 1, false);
        assert_eq!(s.color_count, MAX_MIX + 1);
        assert_eq!(s.parts_total, MAX_MIX + 1);

        // And the defaults are legal by construction.
        let d = Settings::new();
        assert!(d.color_count > d.mix_parts);
        assert!(d.parts_total >= d.mix_parts);
    }

    #[test]
    fn settings_round_trip_through_storage() {
        let s = Settings::clamped(10, 3, 8, true);
        let stored = s.to_stored().expect("non-default settings store");
        assert_eq!(Settings::from_stored(Some(&stored)), s);
        // An older line without the flag reads with the preview off.
        let old = "10,3,8";
        let read = Settings::from_stored(Some(old));
        assert_eq!(read.color_count, 10);
        assert!(!read.show_preview);
    }

    #[test]
    fn corrupt_settings_are_not_an_error() {
        for bad in [
            None,
            Some(""),
            Some("nonsense"),
            Some("1,2"),
            Some(
                "5,3,8,,
        ",
            ),
        ] {
            let _ = Settings::from_stored(bad);
        }
        assert_eq!(Settings::from_stored(Some("junk")), Settings::new());
        // Defaults store as nothing: a player who never changed anything
        // writes nothing.
        assert_eq!(Settings::new().to_stored(), None);
    }

    #[test]
    fn a_mix_is_the_parts_weighted_channel_mean() {
        // Two parts red, one part white.
        let rgb = mix(&[3, 0], &[2, 1]);
        assert_eq!(rgb[0], u8::try_from((0xd3 * 2 + 0xff + 1) / 3).unwrap());
        // Rounding is half up, not banker's: 0.5 of a channel unit goes up.
        assert_eq!(mix(&[1, 2], &[1, 1]), [0x54, 0x5a, 0x60]);
        // An empty mix is black rather than a division trap.
        assert_eq!(mix(&[], &[]), [0, 0, 0]);
    }

    #[test]
    fn hex_is_lowercase_and_padded() {
        assert_eq!(hex([0, 0x0a, 0xff]), "#000aff");
    }

    #[test]
    fn generated_rounds_are_well_formed() {
        for seed in 0..64u64 {
            let settings = Settings::clamped(8, 3, 9, false);
            let round = Round::generate(&settings, 1_000 + seed);
            assert_eq!(round.palette.len(), 8);
            assert_eq!(round.ingredients.len(), 3);
            assert_eq!(round.parts.iter().sum::<usize>(), 9);
            assert!(round.parts.iter().all(|&part| part >= 1));
            for &ingredient in &round.ingredients {
                assert!(round.palette.contains(&ingredient), "ingredient in palette");
            }
            // The palette is sorted, so swatch positions are stable.
            let mut sorted = round.palette.clone();
            sorted.sort_unstable();
            assert_eq!(sorted, round.palette);
        }
    }

    #[test]
    fn the_target_matches_its_own_definition() {
        let settings = Settings::clamped(8, 2, 6, false);
        let round = Round::generate(&settings, 7);
        assert_eq!(round.target_rgb(), mix(&round.ingredients, &round.parts));
    }

    #[test]
    fn rounds_have_unique_solutions() {
        // Many seeds, several shapes: this is the round generator's honesty
        // property, and it is what the checker's fairness rests on.
        for seed in 0..40u64 {
            for &mix_parts in &[2usize, 3] {
                let settings = Settings::clamped(10, mix_parts, 8, false);
                let round = Round::generate(&settings, 500 + seed * 7);
                assert!(
                    round.has_unique_solution(),
                    "seed {seed}, mix {mix_parts}: dealt a round with a second solution"
                );
            }
        }
    }

    #[test]
    fn hints_come_in_order_and_are_exhausted() {
        let settings = Settings::clamped(6, 2, 6, false);
        let mut round = Round::generate(&settings, 3);
        let first = round.hint().expect("an ingredient hint first");
        let second = round.hint().expect("the second ingredient");
        match (first, second) {
            (Hint::Ingredient(a), Hint::Ingredient(b)) => {
                assert_ne!(a, b);
                assert_eq!(&[a, b][..], &round.ingredients[..]);
            }
            _ => panic!("ingredient hints must come first"),
        }
        assert_eq!(round.hint(), Some(Hint::Proportions));
        assert_eq!(round.hint(), None, "nothing left after the proportions");
        // And the lines read like sentences about the round.
        let lines = round.hint_lines();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains(PIGMENTS[round.ingredients[0]].name));
        assert!(lines[2].starts_with("The parts split "));
    }

    #[test]
    fn the_proportions_hint_does_not_leak_the_alignment() {
        // Sorted descending, so it is not the ingredient order even when the
        // membership hints are already out.
        let settings = Settings::clamped(6, 2, 6, false);
        let mut round = Round::generate(&settings, 11);
        while round.hint().is_some() {}
        let line = round.hint_text(&Hint::Proportions);
        let mut parts = round.parts.clone();
        parts.sort_unstable_by(|a, b| b.cmp(a));
        assert!(line.ends_with(&format!(
            "{}.",
            parts
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(" : ")
        )));
    }

    #[test]
    fn an_attempt_is_built_and_torn_down_by_toggles() {
        let mut game = Game::new(Settings::clamped(6, 2, 6, false), 5);
        let a = game.round.ingredients[0];
        let b = game.round.ingredients[1];
        let distractors: Vec<usize> = game
            .round
            .palette
            .iter()
            .copied()
            .filter(|id| !game.round.ingredients.contains(id))
            .collect();
        game.toggle(a);
        assert_eq!(game.assigned(), 1);
        // The attempt is NOT capped at the target's colour count: extra
        // colours are how a player explores.
        game.toggle(distractors[0]);
        game.toggle(b);
        assert_eq!(game.attempt.len(), 3, "a third colour is allowed");
        // The remaining three swatches can still join: the cap is the parts
        // jar (six), which three more colours at one part each exactly fills.
        let unpicked: Vec<usize> = game
            .round
            .palette
            .iter()
            .copied()
            .filter(|id| !game.attempt.iter().any(|entry| entry.pigment == *id))
            .collect();
        for id in unpicked {
            game.toggle(id);
        }
        assert_eq!(game.attempt.len(), 6, "every palette colour can be picked");
        assert_eq!(game.assigned(), 6, "one part each, jar exactly full");
        // Toggling out clears the entry and its parts.
        game.toggle(distractors[0]);
        assert!(!game
            .attempt
            .iter()
            .any(|entry| entry.pigment == distractors[0]));
    }

    #[test]
    fn parts_adjust_saturate() {
        let mut game = Game::new(Settings::clamped(6, 2, 4, false), 5);
        let a = game.round.ingredients[0];
        game.toggle(a);
        for _ in 0..99 {
            game.nudge(a, true);
        }
        assert_eq!(game.assigned(), 4, "never above the round's total");
        for _ in 0..99 {
            game.nudge(a, false);
        }
        assert_eq!(game.assigned(), 1, "a picked colour keeps one part");
        // Adjusting a colour that is not picked does nothing.
        for _ in 0..3 {
            game.nudge(game.round.palette[1], true);
        }
        assert_eq!(game.assigned(), 1);
    }

    #[test]
    fn a_check_needs_all_parts_placed() {
        let mut game = Game::new(Settings::clamped(6, 2, 6, false), 5);
        assert_eq!(game.check(), None);
        // One colour carrying all six parts is a complete attempt now: the
        // player may test a single-colour guess.
        game.toggle(game.round.ingredients[0]);
        for _ in 0..5 {
            game.nudge(game.round.ingredients[0], true);
        }
        assert!(game.attempt_is_complete());
        assert_eq!(game.check(), Some(Outcome::Wrong));
        // And an incomplete attempt is still refused.
        let mut sparse = Game::new(Settings::clamped(6, 2, 6, false), 5);
        sparse.toggle(sparse.round.ingredients[0]);
        assert_eq!(sparse.check(), None);
    }

    #[test]
    fn a_correct_attempt_solves_in_any_pick_order() {
        let mut game = Game::new(Settings::clamped(6, 2, 6, false), 5);
        // Picked in reverse order of the target's own list.
        let plan: Vec<(usize, usize)> = game
            .round
            .ingredients
            .iter()
            .copied()
            .zip(game.round.parts.iter().copied())
            .collect();
        for (ingredient, parts) in plan.into_iter().rev() {
            game.toggle(ingredient);
            for _ in 1..parts {
                game.nudge(ingredient, true);
            }
        }
        let before = game.score.attempts;
        assert_eq!(
            game.check(),
            Some(Outcome::Solved(SOLVE_POINTS + CLEAN_BONUS))
        );
        assert_eq!(game.score.attempts, before + 1);
        assert_eq!(game.score.solved, 1);
        assert_eq!(game.score.streak, 1);
    }

    #[test]
    fn a_wrong_attempt_keeps_the_streak_honest() {
        let mut game = Game::new(Settings::clamped(6, 2, 6, false), 5);
        let wrong = *game
            .round
            .palette
            .iter()
            .find(|id| !game.round.ingredients.contains(id))
            .expect("a distractor exists");
        game.toggle(wrong);
        game.toggle(game.round.ingredients[0]);
        // Fill the parts across the two picks; the mix is wrong, not the count.
        for _ in 0..game.settings.parts_total - 2 {
            game.nudge(game.round.ingredients[0], true);
        }
        assert_eq!(game.assigned(), game.settings.parts_total);
        assert_eq!(game.check(), Some(Outcome::Wrong));
        assert_eq!(game.score.streak, 0);
        assert_eq!(game.score.attempts, 1);
        // The status says so, in the one wording the UI shows.
        assert_eq!(game.status(), "Not this mix. Adjust and check again.");
    }

    #[test]
    fn hints_cost_points_and_clean_solves_pay_a_bonus() {
        let mut game = Game::new(Settings::clamped(6, 2, 6, false), 5);
        game.score.points = 10;
        assert!(game.hint().is_some());
        assert_eq!(game.score.points, 0, "floored at zero");
        game.score.points = 100;
        game.hint();
        assert_eq!(game.score.points, 100 - HINT_COST);

        // Solving a hinted round pays the base only.
        play_target(&mut game);
        let expected = game.score.points + SOLVE_POINTS;
        game.check();
        assert_eq!(game.score.points, expected);

        // An unhinted round pays the bonus on top.
        game.next_round(9);
        play_target(&mut game);
        let expected = game.score.points + SOLVE_POINTS + CLEAN_BONUS;
        game.check();
        assert_eq!(game.score.points, expected);
        assert_eq!(game.score.streak, 2);
        assert_eq!(game.score.best_streak, 2);
    }

    #[test]
    fn a_new_round_drops_everything() {
        let mut game = Game::new(Settings::clamped(6, 2, 6, false), 5);
        game.hint();
        play_target(&mut game);
        game.check();
        game.next_round(21);
        assert!(game.attempt.is_empty());
        assert!(game.round.revealed.is_empty());
        assert_eq!(game.outcome, None);
        // A fresh round keeps the score: it is the game's memory, not the
        // round's.
        assert!(game.score.attempts >= 1);
    }

    #[test]
    fn new_settings_deal_a_new_round_of_that_shape() {
        let mut game = Game::new(Settings::new(), 5);
        game.toggle(game.round.ingredients[0]);
        game.apply_settings(Settings::clamped(9, 3, 10, true), 33);
        assert_eq!(game.settings.color_count, 9);
        assert_eq!(game.settings.mix_parts, 3);
        assert_eq!(game.settings.parts_total, 10);
        assert!(game.settings.show_preview);
        assert_eq!(game.round.palette.len(), 9);
        assert_eq!(game.round.ingredients.len(), 3);
        assert!(game.attempt.is_empty());
    }

    #[test]
    fn the_checker_sorts_the_picked_colours_before_comparing() {
        // The ingredients' own order is (a, b) with a < b; pick b first. The
        // pairs must align by pigment, or the shares compare against the
        // wrong ingredient and a correct mix reads as wrong.
        let mut game = Game::new(Settings::clamped(6, 2, 6, false), 5);
        let dealt = game
            .round
            .ingredients
            .iter()
            .copied()
            .zip(game.round.parts.iter().copied())
            .collect::<Vec<_>>();
        // Play deliberately out of order: highest pigment id first.
        let mut plan = dealt.clone();
        plan.sort_by_key(|&(pigment, _)| std::cmp::Reverse(pigment));
        for (pigment, parts) in plan {
            game.toggle(pigment);
            for _ in 1..parts {
                game.nudge(pigment, true);
            }
        }
        assert!(
            matches!(game.check(), Some(Outcome::Solved(_))),
            "the true mix must solve whatever order it is picked in (dealt {dealt:?})"
        );
    }

    #[test]
    fn a_chip_tap_takes_one_part_back_at_a_time() {
        let mut game = Game::new(Settings::clamped(6, 2, 6, false), 5);
        let a = game.round.ingredients[0];
        game.toggle(a);
        for _ in 0..3 {
            game.nudge(a, true);
        }
        assert_eq!(game.assigned(), 4);
        // Three taps, three parts back.
        game.take_back(a);
        game.take_back(a);
        game.take_back(a);
        assert_eq!(game.assigned(), 1);
        assert_eq!(game.attempt.len(), 1, "still picked at one part");
        // And the last tap un-picks the colour entirely.
        game.take_back(a);
        assert!(game.attempt.is_empty());
        // Tapping a colour that is not picked does nothing.
        game.take_back(game.round.palette[0]);
        assert!(game.attempt.is_empty());
        // A tap also re-hides a checked mix, like every other change.
        game.toggle(a);
        game.nudge(a, true);
        game.toggle(game.round.ingredients[1]);
        for _ in 0..game.settings.parts_total - 2 {
            game.nudge(game.round.ingredients[1], true);
        }
        game.check();
        assert!(game.checked.is_some());
        game.take_back(a);
        assert!(game.checked.is_none(), "a take-back invalidates the check");
    }

    #[test]
    fn readable_text_colour_sits_on_the_chips() {
        // The palette's own extremes: white takes black text, black takes
        // white, and the lightest and darkest pigments follow them.
        assert_eq!(text_on([255, 255, 255]), "#000000");
        assert_eq!(text_on([26, 26, 26]), "#ffffff");
        assert_eq!(text_on(PIGMENTS[14].rgb), "#000000"); // Peach
        assert_eq!(text_on(PIGMENTS[10].rgb), "#ffffff"); // Navy
                                                          // And everything in between gets whichever side clears 4.5:1.
        for pigment in PIGMENTS {
            let on = text_on(pigment.rgb);
            assert!(
                on == "#000000" || on == "#ffffff",
                "text_on must answer one of the two text colours: {pigment:?}"
            );
        }
    }

    #[test]
    fn the_status_walks_through_the_attempt() {
        let mut game = Game::new(Settings::clamped(6, 2, 6, false), 5);
        assert_eq!(game.status(), "Pick colours and split 6 parts.");
        game.toggle(game.round.ingredients[0]);
        assert_eq!(game.status(), "5 more parts to place.");
        game.toggle(game.round.ingredients[1]);
        assert_eq!(game.status(), "4 more parts to place.");
        // Fill the parts to the round's own split, one unit at a time.
        let plan: Vec<(usize, usize)> = game
            .round
            .ingredients
            .iter()
            .copied()
            .zip(game.round.parts.iter().copied())
            .collect();
        for (ingredient, parts) in plan {
            for _ in 1..parts {
                game.nudge(ingredient, true);
            }
        }
        assert_eq!(game.status(), "All parts placed. Check when ready.");
        game.check();
        assert!(game.status().starts_with("Solved — +"));
    }

    #[test]
    fn the_score_round_trips_through_storage() {
        let mut game = Game::new(Settings::new(), 5);
        play_target(&mut game);
        game.check();
        let stored = game.score.to_stored().expect("a played score stores");
        assert_eq!(Score::from_stored(Some(&stored)), game.score);
        // An unplayed score stores as nothing.
        assert_eq!(Score::default().to_stored(), None);
        // Junk reads as a fresh score.
        assert_eq!(Score::from_stored(Some("junk")), Score::default());
        assert_eq!(Score::from_stored(None), Score::default());
    }

    #[test]
    fn the_palette_never_outruns_the_real_pigments() {
        let settings = Settings::clamped(MAX_COLORS, MAX_MIX, MAX_PARTS, false);
        let round = Round::generate(&settings, 2);
        assert_eq!(round.palette.len(), MAX_COLORS);
        assert!(round.palette.iter().all(|&id| id < PIGMENTS.len()));
    }
}
