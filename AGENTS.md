# color_mix

A colour-mixing practice game, and a member of the family of small Rust PWAs.
Each round hides a target colour that is some `mix_parts` of the offered
swatches in some split of `parts_total` parts; the player picks the colours,
splits the parts, and checks. Hints reveal one property at a time and cost
points. Generated from `pwa_template`; the shape and the rules are the
template's, so `INSTRUCTIONS.md` (inherited verbatim) is the coding contract
and this file only adds what is specific to this app.

AGPL-3.0-only: see `LICENSE`.

**This repository is retired, and nothing new should land here.** The game moved
to the managed repository `color-mixing`, which is a fork of the public
`Zuuja/color-mixing` whose `main` publishes the site; work happens there, in a
feature copy of that repository, and reaches the site as a pull request from the
fork. `worktrees/color_mixing/master` stays as the last state of this history and
is what `/color_mix/` was served from before the move. The two workflows here
are the same split the move carries: `.github/workflows/build.yml` runs
read-only on every push and pull request, `.github/workflows/pages.yml` builds
the same site and publishes it from a trunk branch of the repository that owns
it, behind `pages: write` and the `github-pages` environment. `tests/shell.rs`
asserts that split, both trunk names, the deploy gate, and the byte-identical
site check.

## The game's shape

`src/mix.rs` owns everything: the sixteen-pigment palette (array index = the
pigment's id everywhere; never reorder it — swatch positions depend on it),
`Settings` (colour count, mix parts, parts total, preview flag; clamped so
the palette always keeps a distractor and the parts always cover every
ingredient), `Round` (palette, ingredients, parts, revealed hints),
`Game` (round + attempt + score), and every user-visible word
(`status`/`round_line`/`score_line`/`next_label`/`hint_text`). `src/ui.rs`
only writes those words into the DOM.

Two rules are worth knowing before editing:

- **Rounds are dealt unique.** `Round::generate` rejects — and redraws, to a
  bounded 256 attempts — any candidate whose target can also be mixed from a
  different choice of colours or parts, because the player answers by eye and
  a second visually identical answer would score a correct-looking attempt as
  wrong. The prune (`hull_contains`) skips subsets whose channel ranges
  cannot span the target before enumerating compositions. `rounds_have_
  unique_solutions` in `mix.rs` holds the generator to this over many seeds.
- **The reveal rule is the preview setting's whole point.** With *show my mix*
  off, the attempt's colour is the answer to a check: `Game::reveal_attempt`
  allows showing it only while the attempt is byte-identical to the snapshot
  the last check took (`Game::checked`), and any toggle or nudge hides it
  again. With it on, the mix shows as it is built. The UI must not make this
  decision from class names; it calls `reveal_attempt()`.

Hints reveal the ingredients one at a time (in the target's own order) and
the part split last, sorted descending, so the last hint is a check rather
than a reveal. Each reveal costs `HINT_COST` points, floored at zero; a clean
solve pays `CLEAN_BONUS` on top of `SOLVE_POINTS`.

## Build and verify

Two builds, in the documented order (see `INSTRUCTIONS.md` and the workflow):

```
cargo build --locked --lib --target wasm32-unknown-unknown --release
wasm-bindgen --target web --no-typescript --out-dir dist --out-name app \
  target/wasm32-unknown-unknown/release/color_mix.wasm
touch build.rs && cargo build --release --locked
```

`wasm-bindgen` is pinned `=0.2.128` in `Cargo.toml` and the CLI must match
exactly (a mismatched generator fails at page load, not at compile time).
`cargo build` writes the nine-file site into `dist/`; nothing generated is
committed.

```
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --lib --target wasm32-unknown-unknown -- -D warnings
cargo fmt --check
```

`tests/shell.rs` is the template's executable specification adapted to this
app: the file-list/allowlist invariants, the workflow assertions (read from
`build.yml` and `pages.yml` with comments stripped), the icon digest pin
(`6f985823…`, two drops meeting, in the family grey `#444444`), and this
app's own additions — the preview rule living in the domain module, and every
game word rendered from it rather than composed in the UI. Source-text tests
strip comments first, and the workflow ones read values out rather than
searching for fixed strings; keep both properties when adding tests, and
mutation-check anything new (comment the line out, watch the test go red).

## Serving

`/color_mix/` is now served from the released `dist/` of `color-mixing` (see
the host's `apps.json`), not from here. Every URL in the site is relative, so
the same build works under any other prefix or file host.
