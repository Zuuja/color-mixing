# Colour Mix

A colour-mixing practice game: each round shows a target colour and a palette
of swatches, and you work out which swatches make it — and in what
proportions. Pick the colours, split the parts, and check your mix. Stuck?
Ask for a hint: it reveals one property at a time and costs points.

Everything is configurable: how many swatches to choose from, how many
colours the target mixes, how many parts to split, and whether your mix is
shown while you build it or only once you check it. Generated from the Rust
PWA template: all the logic, state and rendering are Rust, the HTML is a
static shell, and `cargo build` writes a publishable site into `dist/`.

## Build

Two steps, because there are two targets. Nothing generated is committed.

```bash
rustup target add wasm32-unknown-unknown

# 1. the site: compile to wasm and generate the JS bindings
cargo build --locked --lib --target wasm32-unknown-unknown --release
wasm-bindgen --target web --no-typescript --out-dir dist --out-name app \
  target/wasm32-unknown-unknown/release/color_mix.wasm

# 2. the rest of the site
touch build.rs && cargo build --release --locked
```

`wasm-bindgen` must match the `=0.2.128` pin in `Cargo.toml` exactly.
`cargo test --locked` needs none of the above.

## Publish

`.github/workflows/build.yml` builds the site on every push and pull request
and runs a check on the `dist/` it produced; it publishes nothing.
`.github/workflows/pages.yml` runs the same build and check, then deploys to
GitHub Pages — from a trunk branch of this repository only, and never from a
fork of it. Enable Pages on the repository (source: GitHub Actions) once, and
every merge to the trunk publishes the site.

## Layout

    src/mix.rs             the game: palette, rounds, attempts, hints, score
    src/ui.rs              the browser layer: the only file that touches the DOM
    src/ui.html            the static shell
    src/style.css          one stylesheet, dark-first with a light override
    src/service-worker.js  install, activate, fetch; one __VERSION__ placeholder
    build.rs               writes six of the nine files in dist/
    assets/icon.svg        the icon: two drops meeting, committed and authoritative
    tests/shell.rs         invariants of the committed shell and the workflow

## Licence

AGPL-3.0-only. See `LICENSE`.
