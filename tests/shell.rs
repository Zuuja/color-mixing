//! Invariants of the committed shell, and of what is and is not committed.
//!
//! Everything here reads committed files. `dist/` is build output and is
//! gitignored, so the release gate — which exports the candidate tree — never
//! has it and cannot build it: that needs the wasm target and a pinned
//! `wasm-bindgen` CLI. A test asserting on `dist/` would therefore run only in
//! a developer's checkout, which is exactly where it is least likely to catch
//! anything. What covers the built output is running the two build steps in the
//! order AGENTS.md gives them, and `.github/workflows/build.yml` does that on
//! every push.
//!
//! What these tests do protect is that the shell cannot quietly grow
//! application JavaScript again, cannot point anywhere absolute, cannot commit a
//! generated file, and cannot lose the icon that every install icon is derived
//! from. Each of those has been true of a real version of this repository.

use std::fmt::Write as _;
use std::path::Path;

/// The repository root, as the tests see it.
fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The app shell, as committed.
///
/// `build.rs` copies this into `dist/index.html` byte for byte, so asserting on
/// it asserts on exactly what gets published.
fn shell() -> String {
    let path = root().join("src/ui.html");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()))
}

/// The service worker template, as committed.
/// The committed stylesheet, as the build publishes it.
fn style() -> String {
    let path = root().join("src/style.css");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()))
}

fn worker_template() -> String {
    let path = root().join("src/service-worker.js");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()))
}

/// Tracked file names, or `None` outside a checkout.
///
/// The release gate exports the candidate as a bare directory with no `.git`,
/// so there is no index to ask. Callers decide what that means.
fn tracked_files() -> Option<String> {
    let inside = std::process::Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(root())
        .output()
        .is_ok_and(|out| out.status.success());
    if !inside {
        return None;
    }
    let output = std::process::Command::new("git")
        .args(["ls-files"])
        .current_dir(root())
        .output()
        .expect("git ls-files");
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Every file `dist/` must contain once both build steps have run.
///
/// The two `wasm-bindgen` artefacts are named here because nothing in the tree
/// generates them; the other six are the files `build.rs` owns, and it asserts
/// those itself by parsing the worker's allowlist (see
/// `every_precached_file_is_published_by_the_build_script`). This list is what
/// the workflow's site check uses, and it is written out by name on purpose: a
/// count would not catch a file that is referenced but never written.
const DIST_FILES: [&str; 9] = [
    "app.js",
    "app_bg.wasm",
    "icon.svg",
    "icon-192.png",
    "icon-512.png",
    "index.html",
    "manifest.webmanifest",
    "service-worker.js",
    "style.css",
];

/// The files `build.rs` publishes, read out of the script itself.
///
/// Derived, never duplicated: a hand-kept list of eight names is a second copy
/// of the truth, and it drifts — which is exactly how `icon.svg` came to be
/// referenced by the page and the worker but never written to `dist/`. A test
/// that duplicated the list would have agreed with itself and passed.
fn published_by_build_script() -> Vec<String> {
    let build = std::fs::read_to_string(root().join("build.rs")).expect("the build script");

    // The verbatim copies: `SHELL` names the destination first.
    let (_, shell) = build
        .split_once("const SHELL: &[(&str, &str)] = &[")
        .expect("build.rs declares the files it copies");
    let shell = shell
        .split_once("];")
        .expect("the end of the SHELL table")
        .0;
    let mut names: Vec<String> = shell
        .lines()
        .filter_map(|line| line.trim().strip_prefix("(\""))
        .filter_map(|rest| rest.split('"').next())
        .map(str::to_owned)
        .collect();

    // The icon is published under a constant, beside the PNGs derived from it.
    // This must come from the push, not from the constant's declaration: reading
    // the constant would accept a site that publishes the icon under a name
    // built at runtime, and would then agree with itself about a file the worker
    // cannot fetch. What has to be true is that *the bytes* reach `dist/` under
    // a name the allowlist uses.
    let mut published_icons: Vec<&str> = build
        .lines()
        .filter(|line| {
            line.trim_start().starts_with("built.push((") && line.contains("icon-{size}.png")
        })
        .filter_map(|line| line.split('"').nth(1))
        .collect();
    // Each entry must name the SVG as its content, and the allowlist is the
    // arbiter of what that name is, so the check below closes the loop.
    assert!(
        build.lines().any(|line| {
            line.trim_start().starts_with("built.push((")
                && line.contains("svg.clone()")
                && !line.contains("format!(\"icon-")
        }),
        "build.rs derives the install icons but never publishes assets/icon.svg itself; \
         the page links it as the favicon and the worker precaches it"
    );
    published_icons.sort_unstable();
    names.extend(published_icons.into_iter().map(str::to_owned));

    // And the icon itself. Its published name is decided by `ICON_OUT`, so read
    // the constant's *value* here: scanning for the constant's name instead
    // would happily accept `format!("{ICON_OUT}.bak")`, which writes the SVG
    // under a name no allowlist can fetch — and then agree with itself.
    let icon_out = build
        .lines()
        .find_map(|line| line.trim().strip_prefix("const ICON_OUT: &str = \""))
        .and_then(|rest| rest.split('"').next())
        .expect("build.rs declares the icon's published name")
        .to_owned();
    names.push(icon_out);

    // The manifest, assembled in Rust.
    if build.contains("\"manifest.webmanifest\"") {
        names.push("manifest.webmanifest".to_owned());
    }

    // The install PNGs, named with a format string at each declared size.
    let declared = build
        .lines()
        .find_map(|line| line.trim().strip_prefix("const ICON_SIZES: [u32; 2] = ["))
        .and_then(|rest| rest.split(']').next())
        .unwrap_or_default();
    for size in declared
        .split(',')
        .filter_map(|size| size.trim().parse::<u32>().ok())
    {
        names.push(format!("icon-{size}.png"));
    }

    // The service worker, written from the template.
    if build.contains("template.replace(\"__VERSION__\"") {
        names.push("service-worker.js".to_owned());
    }

    names.sort();
    names
}

/// The entries of the worker's `ASSETS` allowlist, parsed out of the template.
///
/// These are the URLs `caches.addAll` fetches on install. `addAll` rejects the
/// whole install if any one of them 404s, so a name that is allowlisted but
/// never published does not degrade to a missing icon: it takes the service
/// worker down with it, and in the browser it presents as "offline is broken"
/// with nothing in the build output pointing at the missing file.
fn precached_by_the_worker() -> Vec<String> {
    let worker = worker_template();
    let list = worker
        .split_once("const ASSETS = [")
        .expect("the worker declares an ASSETS allowlist")
        .1
        .split_once(']')
        .expect("the end of the ASSETS allowlist")
        .0;
    list.split('\'')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

/// The manifest is what the browser reads to decide what an install *is*, so
/// it is the one precached URL the worker must never answer from the cache: a
/// cached copy would let a fresh install re-derive the app's identity from a
/// manifest older than the last change to it — the exact shape of the bug
/// that once made every install in this family claim the same identity. It
/// still sits in `ASSETS`, so a manifest that 404s fails the install loudly;
/// it just always reaches the network.
#[test]
fn the_worker_never_serves_the_manifest_from_the_cache() {
    let worker = worker_template();
    assert!(
        precached_by_the_worker()
            .iter()
            .any(|asset| asset == "manifest.webmanifest"),
        "the manifest must stay in ASSETS: a broken manifest must fail the install loudly"
    );
    let fetch = worker
        .split_once("addEventListener('fetch'")
        .expect("the worker must handle fetch")
        .1;
    assert!(
        fetch.contains("url === MANIFEST"),
        "the fetch handler must exempt the manifest before ever touching the cache; \
         a cached manifest lets an install re-derive a stale identity"
    );
}

/// Every URL the worker precaches is a file `build.rs` publishes.
///
/// This is the assertion that would have caught the missing `icon.svg`, and it
/// is deliberately derived from both sides rather than checked against a third
/// hand-written list: the worker's allowlist says what is fetched, `build.rs`
/// says what is written, and this is where they are made to agree.
#[test]
fn every_precached_file_is_published_by_the_build_script() {
    let published = published_by_build_script();
    assert!(
        !published.is_empty(),
        "could not read the published files out of build.rs; the assertion would pass vacuously"
    );
    assert!(
        published.iter().any(|name| name == "index.html"),
        "build.rs no longer publishes the shell: {published:?}"
    );

    for entry in precached_by_the_worker() {
        if entry == "./" {
            // The scope root, which the host serves as `index.html`.
            continue;
        }
        if WASM_BINDGEN_FILES.contains(&entry.as_str()) {
            assert!(
                !published.contains(&entry),
                "'{entry}' is written by wasm-bindgen, so build.rs must not also write it"
            );
            continue;
        }
        assert!(
            published.contains(&entry),
            "the service worker precaches '{entry}' but build.rs does not publish it; \
             caches.addAll rejects the entire install when any listed URL is missing, \
             so this costs the app its offline support rather than one icon. \
             build.rs publishes: {published:?}"
        );
    }
}

/// Nothing `build.rs` publishes is missing from the allowlist: a published
/// file the worker does not precache is a file the app still needs online.
#[test]
fn everything_the_build_publishes_is_precached_or_understood() {
    let precached = precached_by_the_worker();
    for name in published_by_build_script() {
        if name == "service-worker.js" {
            // A worker is fetched by the browser, never from its own cache.
            continue;
        }
        assert!(
            precached.contains(&name),
            "build.rs publishes '{name}' but the service worker does not precache it"
        );
    }
}

/// `dist/` is the union of the two owners, and the worker allowlists exactly
/// that. This is the test that reports a site in terms a reader can act on:
/// nine files, nine precached, nothing referenced and unwritten.
#[test]
fn the_whole_site_is_the_union_of_its_two_owners() {
    let mut site = published_by_build_script();
    site.extend(WASM_BINDGEN_FILES.iter().map(|name| (*name).to_owned()));
    site.sort();
    site.dedup();

    let mut documented: Vec<String> = DIST_FILES.iter().map(|name| (*name).to_owned()).collect();
    documented.sort();
    assert_eq!(
        site, documented,
        "the files the two build steps write must match the documented site exactly"
    );

    let precached = precached_by_the_worker();
    assert_eq!(
        precached.len(),
        DIST_FILES.len(),
        "the worker precaches every file in the site except itself"
    );
}

/// The two files `wasm-bindgen` writes into `dist/`, and `build.rs` does not.
///
/// They are build output with no committed source, which is exactly why they
/// are named here rather than left implicit: everything else in `dist/` belongs
/// to `build.rs`, and the assertions below are only meaningful if the two
/// ownership domains are both accounted for.
const WASM_BINDGEN_FILES: [&str; 2] = ["app.js", "app_bg.wasm"];

/// Run `git check-ignore` for `path`, or `None` outside a checkout.
fn is_ignored(path: &str) -> Option<bool> {
    tracked_files()?;
    Some(
        std::process::Command::new("git")
            .args(["check-ignore", "-q", path])
            .current_dir(root())
            .status()
            .expect("git check-ignore")
            .success(),
    )
}

/// The app is wasm. A hand-written ABI would mean the Rust in the page is not
/// the Rust in `src/`, which is the one thing this shape exists to prevent.
#[test]
fn the_page_loads_generated_bindings_not_a_manual_wasm_abi() {
    let page = shell();
    assert!(
        page.contains("<!DOCTYPE html>"),
        "the shell must be a document"
    );
    assert!(page.contains("<script type=\"module\">"), "a module script");
    assert!(
        page.contains("import('./app.js')"),
        "the page must load the generated bindings"
    );
    assert!(
        page.contains(".then(m => m.default())"),
        "the loader must hand over to the generated start function"
    );
    assert_eq!(
        page.matches("<script").count(),
        1,
        "exactly one script tag:\n{page}"
    );
    for obsolete in [
        "instantiateStreaming",
        "alloc_buf",
        "wasm.exports",
        "fetch(",
    ] {
        assert!(!page.contains(obsolete), "{obsolete} in the static shell");
    }
}

/// The loader's only job is to hand over and to say so when it cannot. There
/// must be no application logic left in it: a `addEventListener`, a state
/// variable or a second statement doing real work is how a JavaScript app grows
/// back.
#[test]
fn the_loader_does_nothing_but_load_and_report_failure() {
    let page = shell();
    let script = page
        .split_once("<script type=\"module\">")
        .expect("the module script")
        .1
        .split_once("</script>")
        .expect("the end of the module script")
        .0;
    assert!(
        script.contains("catch"),
        "the loader must handle the app failing to load"
    );
    assert!(
        script.contains("getElementById('error')"),
        "the failure branch must show something the user can read"
    );
    assert!(
        script.contains("could not start"),
        "the failure message must say the app could not start"
    );
    // It may declare locals in the failure branch, but it may not do anything
    // the app's logic could ever want: those would be state and events, which
    // are Rust's job. No comment either — a loader that explains itself is
    // usually a loader that grew.
    for forbidden in [
        "//",
        "/*",
        "addEventListener",
        "querySelector",
        "localStorage",
        "function",
        "fetch(",
    ] {
        assert!(
            !script.contains(forbidden),
            "{forbidden} in the loader:\n{script}"
        );
    }
    assert!(
        !script.contains("let ") && script.matches("const ").count() <= 1,
        "the loader must declare no state of its own:\n{script}"
    );
}

/// An installable, accessible page: the install icon is linked, the dynamic
/// readout is announced, and the control is a real button with a visible focus
/// ring and a large enough target.
#[test]
fn the_page_is_accessible_and_installable() {
    let page = shell();
    for required in [
        "id=\"round-line\"",
        "id=\"score-line\"",
        "id=\"status\"",
        "id=\"palette\"",
        "id=\"picks\"",
        "id=\"hint-button\"",
        "id=\"check-button\"",
        "id=\"next-button\"",
        "id=\"color-count\"",
        "id=\"mix-parts\"",
        "id=\"parts-total\"",
        "id=\"show-preview\"",
        "id=\"error\"",
        "role=\"status\"",
        "aria-live",
        "aria-label",
        "<button id=\"hint-button\" type=\"button\">",
        "<button id=\"check-button\" type=\"button\">",
        "<button id=\"next-button\" type=\"button\">",
        "rel=\"manifest\"",
        "rel=\"icon\"",
        "rel=\"apple-touch-icon\"",
        "<noscript>",
    ] {
        assert!(page.contains(required), "the shell is missing {required}");
    }
    // The stylesheet is its own file: linked from the shell, precached by the
    // worker and versioned with the rest of the site, rather than inlined where
    // nothing can cache or diff it on its own.
    assert!(
        page.contains("rel=\"stylesheet\" href=\"./style.css\""),
        "the shell must link its stylesheet, relative to itself"
    );
    assert!(
        !page.contains("<style"),
        "the shell must not inline its styles: they live in src/style.css"
    );
    let css = style();
    assert!(
        css.contains("outline: 3px solid"),
        "focus must stay visible"
    );
    assert!(
        css.contains("min-height: 44px"),
        "touch targets must be large enough for mobile use"
    );
    assert!(
        css.contains("prefers-color-scheme") && css.contains("prefers-reduced-motion"),
        "the stylesheet must respect both media queries"
    );
}

/// No visible text may name the implementation.
///
/// An interface that tells the user it is "written in Rust", "compiled to
/// WebAssembly" or "loading the Rust app" is talking to nobody: the user came
/// to use a counter, and how it is built is neither useful nor anything they
/// can act on. It is also the single most common way a demo page ends up looking
/// unfinished, because the sentences are about the author, not the app.
///
/// Implementation vocabulary is fine in comments, docs and `AGENTS.md`. It is
/// not fine in anything a reader can see: markup, text, `alt`, the meta
/// description, or any string Rust writes into the DOM.
#[test]
fn no_visible_text_names_the_implementation() {
    let page = shell();
    // Rendered text only: comments are stripped, and `<style>`/`<script>` bodies
    // are dropped too, since neither is something a reader sees.
    let rendered = visible_text(&page);
    for banned in [
        "rust",
        "webassembly",
        "wasm",
        "bindings",
        "compiled",
        "compile",
    ] {
        assert!(
            !rendered.to_lowercase().contains(banned),
            "the rendered page says \"{banned}\"; user-visible text must not name \
             the implementation — say what the user gets or loses instead. \
             Visible text was:\n{rendered}"
        );
    }
    // "JavaScript" is the single exception, and only in a `<noscript>`: naming
    // the thing the user must go and switch on is the one case where the
    // implementation vocabulary *is* the instruction. Everywhere else it is
    // not actionable and is banned above.
    let outside_noscript = rendered.replace(noscript_body(&rendered).as_str(), "");
    assert!(
        !outside_noscript.to_lowercase().contains("javascript"),
        "only a <noscript> block may name JavaScript; everywhere else it must not"
    );
    // The same rule for every string the Rust layer can write into the DOM.
    let rust = std::fs::read_to_string(root().join("src/ui.rs")).expect("src/ui.rs");
    for line in rust.lines() {
        let trimmed = line.trim_start();
        // Skip doc comments and line comments; they are documentation.
        if !trimmed.starts_with("const ") || !trimmed.contains('"') {
            continue;
        }
        let value = trimmed.split('"').nth(1).unwrap_or_default().to_lowercase();
        for banned in ["rust", "webassembly", "wasm", "javascript", "compiled"] {
            assert!(
                !value.contains(banned),
                "a user-visible string in src/ui.rs says \"{banned}\": {trimmed}"
            );
        }
    }
}

/// A status line must be able to change. An element that can only ever show the
/// text it shipped with is decoration pretending to be information, and two
/// permanently-frozen lines under a working readout are most of what makes a
/// demo page look unfinished.
///
/// The rule this enforces: if a string is only ever set by the loader, it does
/// not belong on the page at all.
#[test]
fn no_status_line_is_frozen() {
    let page = shell();

    // Every `id` the page renders, so a new one cannot slip in unasserted.
    // `#error` is exempt: it is the failure box, filled by the loader's catch
    // and hidden until then. An element that only exists on failure is not a
    // frozen status line.
    let mut shown: Vec<&str> = page
        .match_indices("id=\"")
        .filter_map(|(at, _)| {
            let rest = &page[at + 4..];
            rest.find('"').map(|end| &rest[..end])
        })
        .filter(|id| {
            !matches!(
                *id,
                "round-line"
                    | "score-line"
                    | "status"
                    | "board"
                    | "hints"
                    | "controls"
                    | "settings"
                    | "target-label"
                    | "target-swatch"
                    | "yours-label"
                    | "attempt-swatch"
                    | "attempt-note"
                    | "error"
            )
        })
        .collect();
    shown.sort_unstable();

    let rust = std::fs::read_to_string(root().join("src/ui.rs")).expect("src/ui.rs");
    for id in shown {
        assert!(
            rust.contains(&format!("\"{id}\"")),
            "#{id} is rendered by the shell but src/ui.rs never touches it, so it \
             can only ever show the text it shipped with — remove it or let the \
             app write to it"
        );
    }
}

/// Every control's visible text is its accessible name, and names what it
/// does now. The settings labels are real `<label for>` elements; the three
/// round buttons carry their own text; nothing gets a second, contradicting
/// name.
#[test]
fn every_control_names_what_it_does() {
    let page = shell();
    let rust = std::fs::read_to_string(root().join("src/ui.rs")).expect("src/ui.rs");
    // Strip comments from the shell and take non-comment lines of the Rust,
    // so prose about the rule cannot satisfy an assertion about the rule.
    for (source, text) in [("src/ui.html", &page), ("src/ui.rs", &rust)] {
        for line in text.lines() {
            let trimmed = line.trim();
            // Skip comment lines and the continuation lines of block comments.
            let is_comment = trimmed.starts_with("//")
                || trimmed.starts_with("///")
                || trimmed.starts_with('*')
                || trimmed.starts_with("/*");
            if is_comment {
                continue;
            }
            assert!(
                !trimmed.contains("aria_label") && !trimmed.contains("aria-labelledby"),
                "{source} gives a control a second name in Rust: {trimmed}\n\
                 A control with visible text is named by that text; a second \
                 name contradicts it"
            );
        }
    }

    // The round controls, by name.
    for (id, label) in [("hint-button", "Hint"), ("check-button", "Check")] {
        let needle = format!("<button id=\"{id}\" type=\"button\">");
        let body = page
            .split_once(&needle)
            .unwrap_or_else(|| panic!("the {id} control is in the shell"))
            .1
            .split_once("</button>")
            .expect("the end of the button")
            .0;
        assert_eq!(
            body.trim(),
            label,
            "the {id} control's label must say what it does"
        );
    }

    // The settings labels are bound to their controls.
    for id in ["color-count", "mix-parts", "parts-total", "show-preview"] {
        assert!(
            page.contains(&format!("for=\"{id}\"")),
            "the {id} control has no label bound to it"
        );
    }
}

/// The text inside the page's `<noscript>` element, or `""` if it has none.
fn noscript_body(page: &str) -> String {
    page.split_once("<noscript>")
        .and_then(|(_, rest)| rest.split_once("</noscript>"))
        .map_or(String::new(), |(body, _)| body.to_owned())
}

/// Everything a reader can actually see: `src/ui.html` with its `<!-- -->`
/// comments removed.
///
/// Only HTML comments are dropped; the `<style>` and `<script>` bodies stay,
/// because a `contains` assertion over this text has to be able to see them.
/// The `<noscript>` block is kept for the same reason — a browser with scripting
/// off renders exactly that text, so it is user-visible text and is held to the
/// same rule: it says what the user loses, never what the app is built with.
fn visible_text(page: &str) -> String {
    let mut out = String::with_capacity(page.len());
    let mut rest = page;
    loop {
        let Some(start) = rest.find("<!--") else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..start]);
        rest = rest[start..]
            .find("-->")
            .map_or("", |end| &rest[start + end + 3..]);
    }
}

/// The site is mounted under an arbitrary prefix, so every URL in it is
/// relative. One build, any subdirectory, any file host.
#[test]
fn the_shell_is_mountable_anywhere() {
    let page = shell();
    assert!(
        !page.contains("http://") && !page.contains("https://"),
        "an absolute URL would break the site outside its own origin"
    );
    assert!(
        page.contains("./app.js"),
        "bindings must be referenced relatively"
    );
    assert!(
        page.contains("./icon.svg"),
        "the SVG icon must be linked directly"
    );
    assert!(
        page.contains("./manifest.webmanifest"),
        "the manifest must be linked relatively"
    );
    assert!(
        shell().contains("./icon.svg"),
        "the worker precaches the committed SVG, which is the icon's source of truth"
    );
    // Every file the page links must be one `build.rs` publishes. `./icon.svg`
    // is the one that is easy to miss: it is rasterized, so publishing it looks
    // like an extra step rather than a requirement.
    assert!(
        shell().contains("./icon-192.png") && shell().contains("./manifest.webmanifest"),
        "the page must reference the manifest and the apple-touch icon relatively"
    );
    // The worker's own URLs must resolve the same way.
    assert!(
        worker_template().contains("new URL('./', self.location.href)"),
        "the worker must resolve its cache from its own location"
    );
}

/// The service worker is a committed template with exactly one placeholder, and
/// `build.rs` substitutes it. No placeholder would mean the cache never
/// invalidates; two would mean the substitution is not the only edit.
#[test]
fn the_service_worker_template_has_exactly_one_placeholder() {
    let worker = worker_template();
    assert_eq!(
        worker.matches("__VERSION__").count(),
        1,
        "the template must carry exactly one version placeholder"
    );
    // Everything the worker precaches, including the SVG that is the icon's
    // source of truth and its own `index.html`, so the offline path serves the
    // same page the network path does.
    for file in DIST_FILES
        .iter()
        .filter(|file| **file != "service-worker.js")
    {
        assert!(
            worker.contains(&format!("'{file}'")),
            "the worker does not precache {file}, and a 404 in `caches.addAll` \
             rejects the whole install, so {file} must exist in dist/"
        );
    }
    // The worker is deliberately not in its own list: a worker is fetched by
    // the browser, never from its own cache.
    assert!(
        worker.contains("'icon.svg'"),
        "the worker does not precache the icon's source of truth"
    );
    // The one allowed call is behind the page's `SKIP_WAITING` message: the
    // ask comes from the page that immediately reloads as the worker takes
    // over, so no client ever straddles the two versions. Asserted
    // structurally — the call must sit inside the message handler, gated on
    // the message — so an unprompted or unconditional call cannot creep in.
    let handler = worker
        .split("addEventListener('message'")
        .nth(1)
        .expect("the worker must listen for the page's handover message");
    let handler = &handler[..handler.find("});").expect("the message handler closes")];
    assert!(
        handler.contains("event.data === 'SKIP_WAITING'") && handler.contains("skipWaiting()"),
        "skipWaiting must be the worker's answer to the page's ask, never a call \
         it makes on its own"
    );
    let outside: Vec<&str> = worker
        .split("addEventListener('message'")
        .next()
        .expect("the prefix before the handler")
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .filter(|line| line.contains("skipWaiting"))
        .collect();
    assert!(
        outside.is_empty(),
        "no skipWaiting outside the message handler: an update must never swap \
         the wasm under a live tab unasked, but found {outside:?}"
    );
}

/// The page asks a newly installed worker in, and reloads as it takes over.
///
/// Without the ask, an installed copy only ever moves to a new version when
/// every opening of it has closed — on a phone, possibly never. This pins
/// both halves: the page's message, which the worker answers with
/// `skipWaiting`, and the page's own reload on `controllerchange`.
#[test]
fn the_page_takes_an_installed_worker_over_with_a_reload() {
    let code =
        strip_rust_comments(&std::fs::read_to_string(root().join("src/ui.rs")).expect("src/ui.rs"));
    assert!(
        code.contains("registration.waiting()") && code.contains(".installing()"),
        "the page must look at both workers an update can leave behind: one already \
         installed and waiting, and one discovered while the page stands"
    );
    assert!(
        code.contains("post_message(&JsValue::from_str(\"SKIP_WAITING\"))"),
        "the handover starts with the page's message; the worker never swaps \
         itself in unasked"
    );
    assert!(
        code.contains("controllerchange") && code.contains("location().reload()"),
        "and ends with the page reloading exactly as the worker takes over"
    );
}

/// The worker only ever answers for a URL inside its own app's directory.
///
/// This is the guard that stops one of these apps from taking over the pages it
/// shares an origin with. A service worker registered for a scope is consulted
/// for every URL under that scope, and these apps are all served from the same
/// origin as pages that are not apps at all — so "the scope is small" is a
/// promise, and this test is what keeps it one.
#[test]
fn the_worker_never_answers_outside_its_own_directory() {
    let worker = worker_template();
    assert!(
        worker.contains("new URL('./', self.location.href)"),
        "the worker must resolve its own directory from its location"
    );
    // The guard is a prefix test against that directory, on the request URL,
    // applied before the allowlist decides anything.
    assert!(
        worker.contains("IS_OWN(url)"),
        "the fetch handler must check the request is inside this app's directory; \
         without it a mis-scoped registration serves whatever it cached"
    );
    assert!(
        worker.contains("const IS_OWN = url => url.startsWith(ROOT.href)"),
        "the directory guard must be a prefix test against the worker's own root"
    );
}

/// The page states the worker's scope instead of inheriting it, and cleans up a
/// wider registration left behind by an earlier version.
///
/// A registration outlives the page that created it, and nothing short of an
/// explicit `unregister` takes one away. So the second half is what makes this
/// recoverable without the user clearing their browser: a stale registration
/// is not fixed by a reload, and the newer worker cannot take control of a
/// scope it does not own.
#[test]
fn the_page_states_the_scope_and_releases_a_wider_one() {
    let source = std::fs::read_to_string(root().join("src/ui.rs")).expect("reading src/ui.rs");
    // Comments go first — see `strip_rust_comments`.
    let ui = strip_rust_comments(&source);
    assert!(
        ui.contains("register_with_options"),
        "the worker must be registered with an explicit scope; left to default, \
         the scope is whatever directory the registering page sits in"
    );
    assert!(
        ui.contains("RegistrationOptions::new()") && ui.contains("set_scope(SCOPE)"),
        "the scope has to be actually stated, not merely a named constant"
    );
    assert!(
        ui.contains("get_registrations") && ui.contains("unregister"),
        "a stale wider registration survives a reload, a version bump and a \
         reinstall; only an explicit unregister clears it"
    );
}

/// A worker's script is compared by suffix, not by `trim_end_matches`.
///
/// `trim_end_matches` strips a *set of characters*, so a directory whose name
/// ends in those letters is silently treated as ours — and a registration
/// belonging to a sibling app would be torn down. This is a regression test for
/// a real bug in the first version of this code.
#[test]
fn the_script_comparison_strips_a_suffix_rather_than_a_character_set() {
    let source = std::fs::read_to_string(root().join("src/ui.rs")).expect("reading src/ui.rs");
    // Comments go first — see `strip_rust_comments`.
    let ui = strip_rust_comments(&source);
    // The prose in this file names the method to explain why it is not used, so
    // the assertion is about code: a call, not the word.
    let calls: Vec<&str> = ui
        .lines()
        .filter(|line| {
            let code = line.split("//").next().unwrap_or(line);
            code.contains("trim_end_matches(")
        })
        .collect();
    assert!(
        calls.is_empty(),
        "`trim_end_matches` strips a character set, not a filename: it would eat \
         any directory ending in those letters and tear down a sibling's worker. \
         Found: {calls:?}"
    );
    assert!(
        ui.contains("strip_suffix(\"service-worker.js\")"),
        "the comparison must strip the one filename it expects"
    );
}

/// The scope is named once, and the page and the worker agree on the directory.
///
/// Two independent resolutions of "where am I" — the page's `./` and the worker's
/// `new URL('./', self.location.href)`. They have to describe the same
/// directory, or the page registers a scope the worker's guard does not match.
#[test]
fn the_scope_is_a_relative_directory_shared_with_the_worker() {
    let source = std::fs::read_to_string(root().join("src/ui.rs")).expect("reading src/ui.rs");
    // Comments go first — see `strip_rust_comments`.
    let ui = strip_rust_comments(&source);
    assert!(
        ui.contains("const SCOPE: &str = \"./\";"),
        "the scope must be the app's own directory, relative — so one build works \
         from any subdirectory"
    );
    assert!(
        worker_template().contains("new URL('./', self.location.href)"),
        "the worker must resolve the same directory the page registered"
    );
}

/// The SVG is the icon. It is the committed source of truth, it is what the
/// page links, and both install PNGs are rasterized from it at build time, so
/// it is never replaced by a PNG and never regenerated.
///
/// The digest is the original template's, recorded so that an "improved" icon
/// is a deliberate act with a visible diff rather than a silent one.
///
/// Deliberately changed once, on 2026-10-01: the file had carried the original
/// template's `P` glyph in dark `#434343`, and three bars in a mid grey replaced
/// it. The pin is the mechanism working as intended — a red test, a visible
/// diff, a reason — so it stays, and the reason moves here with the change
/// rather than being left in a commit message to rot.
const ICON_SHA256: &str = "6f985823cdaf94afd8653764668c31fdfe42595bcd458f1afacd3b5ff75f3172";

#[test]
fn the_icon_is_the_committed_svg() {
    let icon = root().join("assets/icon.svg");
    let bytes =
        std::fs::read(&icon).unwrap_or_else(|error| panic!("reading {}: {error}", icon.display()));
    assert_eq!(
        sha256_hex(&bytes),
        ICON_SHA256,
        "assets/icon.svg is the icon, and a change to it must update ICON_SHA256 \
         deliberately: the SVG is the source every install PNG is derived from"
    );
    // One colour, on a transparent ground — asserted where the colour actually
    // is, not by searching the file for a string. A substring search over the
    // whole SVG passes on this file's own comment, which names the old dark grey
    // in exactly the sentence explaining why it went: a test that would survive
    // the very thing it exists to catch.
    //
    // The colour is the family's, not this file's own discovery, and it is
    // pinned here so a copied app inherits it rather than re-deriving a grey
    // of its own — which is how the family ended up two greys apart. The
    // measurement behind it was done once: #444444 reaches 8.28:1 on a white
    // launcher and 1.95:1 on a near-black one — below the 3:1 bar for a
    // graphical object on dark, accepted family-wide for one colour
    // everywhere. INSTRUCTIONS.md carries the rule; this is the enforcement.
    let icon_text = String::from_utf8_lossy(&bytes);
    let fills: Vec<&str> = icon_text
        .match_indices("fill=\"")
        .filter_map(|(at, _)| {
            let rest = &icon_text[at + 6..];
            rest.find('"').map(|end| &rest[..end])
        })
        .collect();
    assert_eq!(
        fills,
        ["#444444"],
        "the icon is one colour, and it is the family's pinned `#444444` — \
         INSTRUCTIONS.md's rule, enforced here so no app re-derives a grey of \
         its own; found fills {fills:?}"
    );
    // The PNGs are build output: never committed, and never in the source tree
    // at all. They exist in `dist/`, which is the site and nothing else. The
    // SVG is the opposite: committed, authoritative, and published to `dist/`
    // unchanged.
    let Some(tracked) = tracked_files() else {
        return; // not a checkout: the gate's exported tree
    };
    assert!(
        tracked.lines().any(|line| line == "assets/icon.svg"),
        "assets/icon.svg is the committed source of truth and must be tracked"
    );
    for artefact in ["assets/icon-192.png", "assets/icon-512.png"] {
        assert!(
            !tracked.lines().any(|line| line == artefact),
            "{artefact} is tracked; install icons are derived at build time and must never be committed"
        );
        assert!(
            !root().join(artefact).exists(),
            "{artefact} exists in the source tree; it belongs in dist/ only"
        );
    }
}

/// Nothing generated may be tracked — not the site, not the wasm, not the
/// bindings. This is the test that would have caught them being committed.
#[test]
fn no_build_artifact_is_committed() {
    let Some(tracked) = tracked_files() else {
        return; // not a checkout
    };
    for artefact in [
        "dist",
        "dist/app.js",
        "dist/app_bg.wasm",
        "dist/index.html",
        "dist/service-worker.js",
        "dist/icon-192.png",
        "dist/icon-512.png",
        "dist/manifest.webmanifest",
    ] {
        assert!(
            !tracked.lines().any(|line| line == artefact),
            "{artefact} is tracked; generated artefacts must never be committed"
        );
    }
}

/// `target/` and `dist/` have to be ignored, or a build leaves the next commit
/// dirty and the release gate tries to ship a wasm.
#[test]
fn build_output_is_ignored() {
    let Some(_) = tracked_files() else {
        return; // not a checkout
    };
    for path in ["dist/", "target/"] {
        assert_eq!(is_ignored(path), Some(true), "{path} must be in .gitignore");
    }
}

/// The old JavaScript template's files are gone, and no build tool crept back in
/// to replace them. A copy-and-rename that kept `Makefile` and `eslint.config.mjs`
/// would be two templates in one repository, disagreeing.
#[test]
fn the_old_javascript_template_is_gone() {
    let Some(tracked) = tracked_files() else {
        return; // not a checkout
    };
    for obsolete in [
        "Makefile",
        "eslint.config.mjs",
        "app.js",
        "sw.js",
        "manifest.json",
    ] {
        assert!(
            !tracked.lines().any(|line| line == obsolete),
            "{obsolete} is still tracked; this is the Rust template"
        );
    }
    assert!(
        !root().join("Makefile").exists(),
        "a Makefile would bring back the build-free JS template's tooling"
    );
}

/// The one place the template states its own rules still exists, and it is
/// still a flat list of short imperatives rather than an essay. This is the
/// artefact a copy of this repo is supposed to inherit.
#[test]
fn the_instructions_are_still_a_short_flat_list() {
    let path = root().join("INSTRUCTIONS.md");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
    assert!(
        text.starts_with("# "),
        "INSTRUCTIONS.md must open with a heading"
    );
    let rules: Vec<&str> = text.lines().filter(|line| line.starts_with("- ")).collect();
    // The upper bound has moved twice and the reason has never been a whim.
    // On 2026-10-02 from 90 to 95, when the GitHub Pages rules were added:
    // publishing the site is part of the contract, and the failures it guards —
    // a deploy in the CI build, publishing permissions granted workflow-wide, an
    // `artifact_id` that v5 silently ignores — are all silent. On 2026-10-11 to
    // 100 for the manifest-identity rules (no relative `id`; the worker never
    // serves the manifest from its cache), which guard failures that present as
    // "the wrong app installed" or "the old app came back", never as a build
    // error; the `skipWaiting` rule was rewritten the same day, message-gated,
    // without changing the count. The stylesheet move on 2026-10-22 added the
    // two rules that say where styles live and that the shell links them, which
    // took the list to 97 — inside the ceiling, so the bound itself stayed put.
    assert!(
        rules.len() >= 30 && rules.len() <= 100,
        "the rules must stay a short list, not an essay: {} rules",
        rules.len()
    );
    for required in [
        "- Build",
        "wasm",
        "wasm-bindgen",
        "prefers-color-scheme",
        "prefers-reduced-motion",
        "aria-live",
        "localStorage",
        "tests",
        // The JS-era tooling must be gone, not merely discouraged.
        "Prettier",
        "ESLint",
        "skipWaiting",
    ] {
        assert!(
            text.contains(required),
            "INSTRUCTIONS.md no longer covers {required}"
        );
    }
    // The tooling rules it replaces have to be rules too, or the reader is left
    // with a hole where verification used to be.
    for required in [
        "cargo test",
        "clippy",
        "touch build.rs",
        "dist/",
        "assets/icon.svg",
        // The family-wide rule prompted by a defect the user found: no
        // user-visible text may name the implementation.
        "name the implementation",
    ] {
        assert!(
            text.contains(required),
            "INSTRUCTIONS.md lost its verification guidance for {required}"
        );
    }
    assert!(
        !text.contains("npx "),
        "INSTRUCTIONS.md still sends the reader to npx"
    );
}

/// A workflow file, as committed.
///
/// Nonexistent is not a reason to fail. The release gate exports the candidate
/// tree, and a partial export of a release branch need not carry every workflow;
/// a test that hard-failed on absence would make such an export red for a reason
/// that says nothing about the app. Callers say which they want — and the ones
/// that matter here also run only in a full checkout.
fn workflow(name: &str) -> Option<String> {
    let path = root().join(".github/workflows").join(name);
    std::fs::read_to_string(&path).ok()
}

/// A workflow with its comments removed.
///
/// A `#` inside a quoted string is not a comment, and these workflows quote
/// nothing in the keys they are read for — but the point is not the corner case.
/// It is that commenting a line out instead of deleting it is the exact mutation
/// most likely to be applied to one of the lines below, and a `# RUSTFLAGS:`
/// satisfies a `contains` assertion that reads the raw text. Every assertion
/// about a setting reads this, not the file.
fn strip_yaml_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| match line.find('#') {
            Some(index) => &line[..index],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The workflow's top-level `env:` block, as key → value.
///
/// Reading the block rather than searching the whole file is what lets a test
/// ask "is this set at the *top level*", which is a question about indentation
/// and cannot be answered by `contains`. The block ends at the first column-zero
/// key — `permissions:` here — so a value belonging to some other top-level
/// section cannot be mistaken for one.
fn top_level_env(yaml: &str) -> Option<std::collections::BTreeMap<String, String>> {
    let mut declared = std::collections::BTreeMap::new();
    let mut in_env = false;
    let mut saw_env = false;
    for line in yaml.lines() {
        if line.is_empty() {
            continue;
        }
        if !line.starts_with(' ') {
            in_env = line.trim_start().starts_with("env:");
            saw_env |= in_env;
            continue;
        }
        if !in_env {
            continue;
        }
        let trimmed = line.trim_start();
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        declared.insert(
            key.to_owned(),
            value.trim().trim_matches(['"', '\'']).to_owned(),
        );
    }
    saw_env.then_some(declared)
}

/// `top_level_env` must see the block it claims to, and only that block.
///
/// Every workflow test that asks "is this set for the whole workflow?" reads
/// through this helper, so a helper that quietly returns an empty map would make
/// them all skip — and a skipped test reads the same as a passing one in the
/// summary.
#[test]
fn the_env_reader_reads_the_block_and_stops_at_the_next_section() {
    let env = top_level_env(
        "\
env:
  CARGO_TERM_COLOR: always
  WASM_BINDGEN_VERSION: 0.2.128

permissions:
  contents: read
",
    )
    .expect("the document has an env: block");
    assert_eq!(
        env.get("CARGO_TERM_COLOR").map(String::as_str),
        Some("always"),
    );
    assert_eq!(
        env.get("WASM_BINDGEN_VERSION").map(String::as_str),
        Some("0.2.128"),
    );
    assert!(
        !env.contains_key("contents"),
        "the reader ran past the end of env: into permissions:",
    );
    assert!(
        top_level_env("name: build\non:\n  push:\n").is_none(),
        "a document with no env: must read as None, not as an empty block",
    );
    assert!(
        top_level_env("jobs:\n  env:\n    CARGO_TERM_COLOR: always\n").is_none(),
        "an env: nested under jobs: is not the workflow's env:",
    );
}

/// The read-only build workflow, as committed, with its comments stripped.
///
/// This is the half that runs on every pull request, including from a fork, so
/// it holds no publishing permission. The half that publishes is `pages.yml`,
/// read through [`live_pages_workflow`]; the two run the same build and the same
/// site check, and the tests below hold them to it.
///
/// Every assertion about a setting reads the stripped text, not the file, so
/// commenting a line out satisfies nothing.
fn live_build_workflow() -> Option<String> {
    workflow("build.yml").map(|text| strip_yaml_comments(&text))
}

/// The publishing workflow, as committed, with its comments stripped.
///
/// Read the same way and for the same reason as [`live_build_workflow`]: a
/// comment above a gate explains what the gate is for by name, so an assertion
/// over the raw text is satisfied by the explanation with the gate deleted.
fn live_pages_workflow() -> Option<String> {
    workflow("pages.yml").map(|text| strip_yaml_comments(&text))
}

/// The workflow must build the site itself, in the documented order, because
/// nothing committed can.
///
/// `dist/app.js` and `dist/app_bg.wasm` exist nowhere else in the tree — they
/// are written by `wasm-bindgen` and are gitignored. A workflow that checked out
/// the repository and uploaded `dist/` would publish a site that loads and never
/// starts, from committed files alone. So the build steps have to be here, in
/// order, and the `touch` has to survive: `build.rs` writes into the source tree
/// rather than `OUT_DIR`, so Cargo cannot see its own output changed and skips
/// the second build, leaving `dist/` with the two wasm artefacts and none of the
/// six shell files.
#[test]
fn the_workflow_builds_the_site_rather_than_publishing_committed_files() {
    let Some(build) = live_build_workflow() else {
        return;
    };
    for needle in [
        "cargo build --locked --lib --target wasm32-unknown-unknown --release",
        "wasm-bindgen --target web --no-typescript --out-dir dist --out-name app",
        "touch build.rs",
        "cargo build --release --locked",
    ] {
        assert!(
            build.contains(needle),
            "build.yml must run `{needle}`; the site's wasm artefacts exist nowhere else"
        );
    }
    // The wasm artefact's name follows the crate, and a renamed crate that
    // forgets this gets an empty bindings directory and a green job.
    let manifest = std::fs::read_to_string(root().join("Cargo.toml")).expect("Cargo.toml");
    let crate_name = manifest
        .lines()
        .find_map(|line| line.trim().strip_prefix("name = "))
        .map(|rest| rest.trim().trim_matches('"').to_owned())
        .expect("Cargo.toml names the package");
    assert!(
        build.contains(&format!(
            "target/wasm32-unknown-unknown/release/{crate_name}.wasm"
        )),
        "build.yml must bind the crate's own wasm ({crate_name}.wasm); a renamed crate that \
         misses this publishes bindings for no crate at all"
    );
}

/// The workflow's generator must be the runtime the crate is built against.
///
/// `Cargo.toml` pins `wasm-bindgen = "=0.2.128"` exactly; the job installs its
/// own CLI from a variable in `env:`. Move the dependency and the workflow keeps
/// building happily: it generates bindings for a runtime the page does not have,
/// and the only symptom is a live site that fails at startup — for every
/// visitor, and only in a browser.
///
/// The *declaration* is asserted as well as the use: an undeclared
/// `WASM_BINDGEN_VERSION` expands to nothing, the generator step installs no
/// generator, and every other check here passes.
#[test]
fn the_build_generates_bindings_for_the_pinned_runtime() {
    let Some(build) = live_build_workflow() else {
        return;
    };
    generator_matches_the_pinned_runtime("build.yml", &build);
}

/// Both workflows install the generator, so both have to install the runtime the
/// crate was compiled against.
///
/// This is the shared body of the two tests that call it. It takes the
/// workflow's name for its messages so a failure names the file that is wrong:
/// `build.yml` runs on every pull request and `pages.yml` is the one that
/// publishes, and a version pin that drifted in only one of them would label the
/// other as the culprit if the message were hard-coded.
fn generator_matches_the_pinned_runtime(workflow_name: &str, build: &str) {
    let manifest = std::fs::read_to_string(root().join("Cargo.toml")).expect("Cargo.toml");
    let pinned = manifest
        .lines()
        .find_map(|line| {
            // The line must *be* the dependency, not merely mention it:
            // `wasm-bindgen-futures = "0.4.78"` sits next to it.
            let rest = line.trim().strip_prefix("wasm-bindgen = ")?;
            let rest = rest.trim().trim_matches('"');
            (!rest.is_empty()).then_some(rest)
        })
        .expect("Cargo.toml pins a wasm-bindgen version");
    assert!(
        pinned.starts_with('='),
        "Cargo.toml must pin wasm-bindgen exactly ({pinned:?}), so there is one version for \
         the crate and one for the bindings generator",
    );
    let version = pinned.trim_start_matches('=');

    let env = top_level_env(build).unwrap_or_else(|| {
        panic!(
            "{workflow_name} must have a top-level `env:` block declaring WASM_BINDGEN_VERSION: \
             {version}; the install step reads $WASM_BINDGEN_VERSION, an undeclared variable \
             expands to nothing, and the job would then generate bindings for no runtime"
        )
    });
    assert_eq!(
        env.get("WASM_BINDGEN_VERSION").map(String::as_str),
        Some(version),
        "{workflow_name}'s WASM_BINDGEN_VERSION must be exactly the Cargo.toml pin ({version})",
    );
    // And the install must read the variable, not a literal.
    let mut uses = build.lines().filter_map(|line| {
        let value = line.trim().strip_prefix("version=")?;
        let value = value.strip_prefix('"')?;
        value.split('"').next()
    });
    let install = uses.next().expect("the install step declares a version");
    assert_eq!(
        install, "$WASM_BINDGEN_VERSION",
        "{workflow_name} must install the generator with version=\"$WASM_BINDGEN_VERSION\" rather \
         than a literal, so the pin in Cargo.toml has exactly one place to be wrong"
    );
    assert_eq!(
        build.matches("version=\"").count(),
        1,
        "{workflow_name} must name the generator version exactly once"
    );
}

/// This crate must not carry, and the workflow must not declare, a compiler
/// flag it does not need.
///
/// Verified before the first commit: a clean wasm build — no `RUSTFLAGS`, no
/// `.cargo/config.toml` — finishes. The cfg is for web-sys's unstable surface,
/// and several web-sys getters are typed differently behind it, so a workflow
/// carrying the cfg the crate is not built with makes the wasm clippy run lint
/// a different program from the one that ships. The rule is the *agreement*,
/// not the absence.
#[test]
fn the_workflow_declines_a_cfg_this_crate_does_not_need() {
    // Both workflows, because they both build the wasm: a flag added to the
    // publishing one alone would make the shipped clippy run lint a different
    // program from the one the pull request checked.
    let mut seen = 0;
    for (name, live) in [
        ("build.yml", live_build_workflow()),
        ("pages.yml", live_pages_workflow()),
    ] {
        let Some(live) = live else {
            continue;
        };
        seen += 1;
        let env = top_level_env(&live).unwrap_or_default();
        assert!(
            !env.contains_key("RUSTFLAGS"),
            "{name} declares RUSTFLAGS={}; this crate uses no unstable web-sys API and its \
             wasm build was verified clean without it, so the flag can only make the wasm \
             clippy run lint a different web-sys than the one that ships",
            env.get("RUSTFLAGS").map_or("", String::as_str),
        );
    }
    // A partial export need not carry either workflow; the manifest and config
    // checks below stand on their own and are the ones that never skip.
    let _ = seen;
    let manifest = std::fs::read_to_string(root().join("Cargo.toml")).expect("Cargo.toml");
    assert!(
        !manifest.contains("rustflags"),
        "Cargo.toml sets rustflags; cargo ignores `[target.*.rustflags]` for registry \
         dependencies, so this would not reach web-sys and only look like it does",
    );
    let config = root().join(".cargo/config.toml");
    assert!(
        !config.exists(),
        ".cargo/config.toml must not exist: this crate needs no compiler flag"
    );
}

/// The site check the workflow runs must assert on the site, not on the exit
/// status.
///
/// No test in this file can assert on `dist/`: it is gitignored, so the release
/// gate's exported tree never has it. The workflow is therefore the only
/// automated coverage of the built output, and it has to check the things a
/// green build cannot imply.
#[test]
fn the_workflow_checks_the_built_site_rather_than_the_exit_status() {
    let Some(build) = live_build_workflow() else {
        return;
    };
    // Every one of the nine files, by name, read out of the workflow's own
    // `expected=` assignment rather than duplicated here.
    let expected = build
        .lines()
        .find_map(|line| {
            // The whole line must be the assignment, not merely contain it:
            // `missing="${missing}"` ends in the same characters.
            let value = line.trim().strip_prefix("expected=")?;
            value.strip_prefix('"')?.strip_suffix('"')
        })
        .expect("build.yml must state the files it expects as an `expected=\"...\"` list");
    let listed: Vec<&str> = expected.split_whitespace().collect();
    assert_eq!(
        listed, DIST_FILES,
        "build.yml's site check names these files; they must be exactly the nine the two \
         owners produce"
    );
    assert!(
        build
            .lines()
            .any(|line| line.contains("grep -x -F -v -f") && line.contains("extra=")),
        "build.yml must reject unexpected files in dist/ as well as missing ones"
    );
    for (needle, why) in [
        (
            "__VERSION__",
            "an unsubstituted placeholder ships a worker that never invalidates",
        ),
        (
            "export",
            "the page loads app.js with a dynamic import, so a truncated one fails only \
                    in a browser",
        ),
        (
            "__wbindgen_start",
            "without it the bindings load and the app never runs, silently",
        ),
    ] {
        assert!(
            build.lines().any(|line| {
                line.contains(&format!("grep -q '{needle}'")) && line.contains("dist/")
            }),
            "build.yml must run a `grep -q '{needle}'` against dist/ ({why})"
        );
    }
    assert!(
        build.lines().any(|line| line.contains("89504e470d0a1a0a")),
        "build.yml must check the PNG magic number; a dist/icon-*.png that is not a PNG has \
         the right name and no image"
    );
    assert!(
        build
            .lines()
            .any(|line| line.contains("grep -q 'm.default()'") && line.contains("dist/index.html")),
        "build.yml must check that dist/index.html calls m.default()"
    );
    assert!(
        build.contains("<<'PY'"),
        "build.yml's multi-line Python must be a <<'PY' heredoc, not python3 -c"
    );
    assert!(
        !build.contains("python3 -c"),
        "build.yml must not use python3 -c for the site check's Python"
    );
}

/// The workflow pins the icon the repository actually has.
///
/// `assets/icon.svg` is the source every install PNG is derived from, and the
/// workflow asserts that `dist/icon.svg` is that file byte for byte at a pinned
/// digest. That check is what caught the template's own workflow sitting red
/// for a day after its icon changed: the same pin, kept in two places, drifting.
///
/// This test reads the digest out of the workflow and asserts it equals the
/// constant the icon test uses, and that the constant still matches the file
/// on disk — so the two cannot drift apart again.
#[test]
fn the_workflow_pins_the_icon_the_repository_still_has() {
    let icon = root().join("assets/icon.svg");
    let bytes =
        std::fs::read(&icon).unwrap_or_else(|error| panic!("reading {}: {error}", icon.display()));
    let digest = sha256_hex(&bytes);
    assert_eq!(
        digest, ICON_SHA256,
        "ICON_SHA256 no longer matches assets/icon.svg; update the constant deliberately"
    );
    let Some(build) = live_build_workflow() else {
        return;
    };
    assert!(
        build.contains(&digest),
        "build.yml pins a digest for assets/icon.svg that is not the icon this repository has \
         (the file hashes to {digest}); the workflow has been red since the icon changed"
    );
}

/// The workflow lints both targets, as AGENTS.md requires.
///
/// The crate denies warnings, but only for the target being compiled, so a
/// clean host build says nothing about the wasm one. A workflow that drops the
/// wasm leg is exactly how a wasm-only regression ships green.
#[test]
fn the_workflow_lints_both_targets() {
    let Some(build) = live_build_workflow() else {
        return;
    };
    assert!(
        build.contains("cargo clippy --locked --all-targets -- -D warnings"),
        "build.yml must lint the host target"
    );
    assert!(
        build
            .contains("cargo clippy --locked --lib --target wasm32-unknown-unknown -- -D warnings"),
        "build.yml must lint the wasm target; a clean host build says nothing about the wasm one"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// The Pages deployment
//
// `dist/` is gitignored and `app.js`/`app_bg.wasm` exist nowhere else in the
// tree, so a Pages build serving committed files cannot publish the game at all:
// the page would load and never start. `.github/workflows/pages.yml` is
// therefore not an optimisation, it is the thing that makes the site exist
// off-host. The assertions below are the invariants that survive it being edited
// by hand, which is what happens to a YAML file that no test reads.
// ─────────────────────────────────────────────────────────────────────────────

/// The publishing workflow installs the generator for the pinned runtime too.
///
/// Same invariant as the build one, checked separately rather than assumed: the
/// two files are edited independently, and the failure is silent in both — a
/// live site that fails at startup, for every visitor, and only in a browser.
#[test]
fn the_pages_build_generates_bindings_for_the_pinned_runtime() {
    let Some(pages) = live_pages_workflow() else {
        return;
    };
    generator_matches_the_pinned_runtime("pages.yml", &pages);
}

/// The site cannot be published by serving committed files, so the workflow that
/// publishes it has to run both builds itself, in order.
///
/// `dist/` is gitignored and two of its files — `app.js` and `app_bg.wasm`,
/// written by `wasm-bindgen` — exist nowhere else in the tree. A workflow that
/// deployed a checked-out `dist/` would publish a page that loads and never
/// starts, and the workflow would be green throughout.
///
/// The order is the second half of the contract: `build.rs` derives the service
/// worker's cache version from the bytes of every other file in `dist/`, the
/// wasm included, so running it first pins the cache to whatever the previous
/// build left behind. The `touch` is not redundancy — `build.rs` writes into the
/// source tree rather than `OUT_DIR`, so cargo cannot see its own output change
/// and skips the second build, leaving `dist/` with the two wasm artefacts and
/// none of the shell files.
#[test]
fn the_pages_workflow_builds_the_site_it_publishes() {
    let Some(pages) = live_pages_workflow() else {
        return;
    };

    let wasm_build = pages
        .find("cargo build --locked --lib --target wasm32-unknown-unknown")
        .expect(
            "pages.yml must build the wasm target: app.js and app_bg.wasm are gitignored build \
             output, and nothing committed produces them",
        );
    let bindings = pages
        .find("wasm-bindgen --target web")
        .expect("pages.yml must run the bindings generator; it is the only writer of dist/app.js");
    let touch = pages.find("touch build.rs").expect(
        "pages.yml must touch build.rs before the host build; build.rs writes into the source \
         tree rather than OUT_DIR, so without the touch the second build is a no-op",
    );
    let host_build = pages
        .find("cargo build --release --locked")
        .expect("pages.yml must run the host build: build.rs writes the shell files from it");
    assert!(
        wasm_build < bindings && bindings < touch && touch < host_build,
        "the two builds must run in order — wasm, bindings, touch build.rs, host. The worker's \
         cache version is hashed from the wasm, so a host build first pins it to whatever the \
         previous build left behind (wasm@{wasm_build}, bindings@{bindings}, touch@{touch}, \
         host@{host_build})",
    );

    // The wasm artefact follows the crate's own name, and a renamed crate that
    // misses this publishes bindings for no crate at all.
    let manifest = std::fs::read_to_string(root().join("Cargo.toml")).expect("Cargo.toml");
    let crate_name = manifest
        .lines()
        .find_map(|line| line.trim().strip_prefix("name = "))
        .map(|rest| rest.trim().trim_matches('"').to_owned())
        .expect("Cargo.toml names the package");
    assert!(
        pages.contains(&format!(
            "target/wasm32-unknown-unknown/release/{crate_name}.wasm"
        )),
        "pages.yml must bind the crate's own wasm ({crate_name}.wasm)",
    );

    // And it must check what it built: the file list it publishes is the same
    // nine the two owners produce, read out of the workflow rather than
    // duplicated here.
    let expected = pages
        .lines()
        .find_map(|line| {
            let value = line.trim().strip_prefix("expected=")?;
            value.strip_prefix('"')?.strip_suffix('"')
        })
        .expect("pages.yml must declare the file list it expects in dist/");
    let listed: Vec<&str> = expected.split_whitespace().collect();
    assert_eq!(
        listed, DIST_FILES,
        "pages.yml's site check must name exactly the nine files the build publishes",
    );
    // The four silent shapes a green build can still be.
    for (needle, why) in [
        (
            "__VERSION__",
            "the worker's cache version would never be substituted",
        ),
        ("'export'", "the page's dynamic import would fail"),
        ("__wbindgen_start", "the game would load and never run"),
        ("m.default()", "the page would never start the game"),
    ] {
        assert!(
            pages.contains(needle),
            "pages.yml must check for {needle} in the built site: {why}",
        );
    }
    assert!(
        pages.lines().any(|line| line.contains("89504e470d0a1a0a")),
        "pages.yml must check the PNG magic number; an icon with the right name and no raster \
         installs as a broken icon",
    );
    assert!(
        pages.contains("<<'PY'") && !pages.contains("python3 -c"),
        "pages.yml's multi-line Python must be a <<'PY' heredoc; an indented python3 -c body is \
         quoted by the shell before Python sees it",
    );
}

/// A deploy that can run from any branch is a deploy a stranger can run.
///
/// `pages: write` and `id-token: write` are the two permissions that let a job
/// overwrite the live site, and the token behind them is minted for the
/// repository however the workflow was reached. Publishing on every merge to a
/// trunk is the point; publishing from anywhere else is not, so the invariant is
/// the narrow one that survives the convenience: a trunk branch is the only ref
/// that reaches the live site, and the publishing permissions live in the one job
/// gated on it.
#[test]
fn only_a_trunk_branch_can_reach_the_live_site() {
    let Some(raw) = workflow("pages.yml") else {
        return;
    };

    // The trigger must name the branches, not be a bare `push:`, which deploys
    // from every branch that exists — including a contributor's. Both trunk
    // names are listed because a fork's own trunk is `main`; what may actually
    // publish is decided by the deploy job's gate below, not here.
    let branches = raw
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("branches:"))
        .expect("pages.yml must state the branches it runs on");
    for branch in ["master", "main"] {
        assert!(
            branches.contains(branch),
            "pages.yml must trigger on {branch} as well as the other trunk name, not a bare \
             `push:`; a bare push deploys from every branch, including other people's (found \
             {branches:?})",
        );
    }
    // No tag trigger: a tagged commit that never reached a trunk would be
    // published.
    assert!(
        !raw.contains("tags:"),
        "pages.yml must not also deploy on tags; a tagged commit that never reached a trunk \
         would be published",
    );

    // The deploy job's own gate, read out with comments stripped: the comment
    // block above the `if:` explains both halves by name, so a plain search over
    // the raw text is satisfied by that explanation with the gate itself deleted.
    let live = strip_yaml_comments(&raw);
    let deploy_job = live
        .split("\n  deploy:")
        .nth(1)
        .expect("pages.yml must have a `deploy:` job");
    let gate = deploy_job
        .split_once("if:")
        .map(|(_, after)| after)
        .expect("the `deploy` job must have an `if:` gate");

    // Half one: a trunk branch, and only a trunk branch.
    for branch in ["master", "main"] {
        assert!(
            gate.contains(&format!("refs/heads/{branch}")),
            "the `deploy` job must be gated on a build for `{branch}`, the branch a merge lands \
             on; the trigger alone is not the gate",
        );
    }

    // Half two: and only in the repository that owns the site. Both of these
    // repositories exist with both trunk names, and `pages.yml` is deliberately
    // identical in them, so a gate that tested only the branch name could not
    // tell them apart. A fork has no Pages site until someone enables one by
    // hand, and until then every push there dies at "Creating Pages deployment
    // failed ... Ensure GitHub Pages has been enabled"; if it were enabled, the
    // fork would serve its own copy and drift from the published site.
    //
    // `github.event.repository.fork` is the discriminator because it needs no
    // configuration: it is supplied by the event, false upstream and true in the
    // fork. A repository Actions variable would have to be set on the upstream
    // repository, whose owner this account is not, so the gate would ship
    // silently off on the one repository where it matters.
    assert!(
        gate.contains("!github.event.repository.fork"),
        "the `deploy` job must be gated on `!github.event.repository.fork`; this workflow is \
         identical in the fork, so a branch-name-only gate tries to publish from both",
    );

    // And the environment, which is what makes the repository's own approval
    // rules able to hold the live site back.
    assert!(
        deploy_job.contains("github-pages"),
        "the deploy must run in the `github-pages` environment, or a protected branch or a \
         reviewer-gated environment cannot stop it",
    );

    // The permissions that can publish belong to that job alone, never
    // workflow-wide: granted globally, a build step or a third-party action
    // added later can spend them, and the build job is the one that runs other
    // people's scripts.
    assert!(
        !live.contains("pages: write") || deploy_job.contains("pages: write"),
        "the workflow declares `pages: write` somewhere other than the deploy job",
    );
    let build_job = live
        .split("\n  build:")
        .nth(1)
        .and_then(|after| after.split("\n  deploy:").next())
        .expect("pages.yml must have a `build:` job");
    assert!(
        !build_job.contains("pages: write") && !build_job.contains("id-token: write"),
        "the `build` job must not hold pages: write or id-token: write; those belong to \
         `deploy`, where the trunk gate is",
    );
}

/// The deploy has to be handed something the upload actually produced.
///
/// `deploy-pages` v5 takes `artifact_name`. There is no `artifact_id` input:
/// passing one is reported as `Unexpected input(s) 'artifact_id'`, warned about,
/// and ignored — the action then falls back to its own default, which is only the
/// right answer while the upload side defaults to the same string.
#[test]
fn the_deploy_is_handed_the_artifact_the_build_uploaded() {
    let Some(pages) = live_pages_workflow() else {
        return;
    };

    assert!(
        !pages.contains("artifact_id:"),
        "pages.yml passes `artifact_id` to deploy-pages v5, which has no such input; it is \
         warned about and ignored, leaving the deploy to guess the artifact name",
    );
    // The Pages artifact is a single tarball the deploy finds *by name*, so a
    // plain `upload-artifact` gives a green build and then a deploy that cannot
    // find what it was given.
    assert!(
        pages.contains("upload-pages-artifact") && !pages.contains("upload-artifact@"),
        "pages.yml must upload with `actions/upload-pages-artifact`, not \
         `actions/upload-artifact`",
    );

    // Both sides name the artifact the same way, read out rather than asserted
    // as a fixed string, so the invariant is the agreement and not the name.
    // Only the artifact's own keys are read: `artifact_name:` is unique, and the
    // upload's `name:` is the one indented ten spaces — a step's own `name:` is
    // eight and the `deploy` job's is four.
    let name_of = |key: &str, indent: usize| {
        pages.lines().find_map(|line| {
            let prefix = format!("{}{key}: ", " ".repeat(indent));
            Some(
                line.strip_prefix(prefix.as_str())?
                    .trim()
                    .trim_matches('"')
                    .to_owned(),
            )
        })
    };
    let uploaded = name_of("name", 10).expect(
        "pages.yml must state the upload step's artifact `name:` so the deploy can match it",
    );
    let deployed = name_of("artifact_name", 10).expect(
        "pages.yml must pass `artifact_name:` to deploy-pages, or it uses a default that can \
         drift from the upload",
    );
    assert_eq!(
        uploaded, deployed,
        "the artifact the build uploads and the one the deploy asks for must be the same name",
    );
    assert_eq!(
        uploaded, "github-pages",
        "the artifact name should stay `github-pages`, the default both actions agree on; \
         renaming it is fine, but rename it on both sides in the same commit",
    );
}

/// The Pages deployment is a separate workflow, not extra steps in `build.yml`.
///
/// `build.yml` runs on every pull request, including from forks, where
/// `pages: write`, `id-token: write` and the `github-pages` environment do not
/// exist. A deploy folded into it is a workflow that can fail to run for every
/// ordinary pull request, and one that is reachable from every branch that can
/// open one. This is the check that the split stays a split.
#[test]
fn the_deploy_is_not_folded_into_the_read_only_build() {
    let Some(build) = live_build_workflow() else {
        return;
    };
    for forbidden in [
        "deploy-pages",
        "upload-pages-artifact",
        "pages: write",
        "id-token: write",
    ] {
        assert!(
            !build.contains(forbidden),
            "build.yml references {forbidden}; it runs on pull requests from forks, where the \
             Pages permissions and environment do not exist, so the deploy must live in \
             pages.yml",
        );
    }
    // And the build it does still has to be able to produce the site, or the
    // split has cost the read-only check rather than adding a second one.
    assert!(
        build.contains("cargo build --locked --lib --target wasm32-unknown-unknown")
            && build.contains("wasm-bindgen --target web")
            && build.contains("cargo build --release --locked"),
        "build.yml must still run both builds itself: the check it runs is the only one that \
         ever sees dist/ on a pull request",
    );
}

/// Both workflows run the same site check, on the same output.
///
/// `build.yml` runs it on every pull request; `pages.yml` runs it again before
/// publishing. Drifted apart, each asserts things the other does not, so a pull
/// request can go green on a site the deploy rejects, and the deploy can publish
/// a file the pull request never checked.
#[test]
fn both_workflows_run_the_same_site_check() {
    let step = |name: &str| -> Option<String> {
        let text = workflow(name)?;
        let start = text.find("      - name: Check the built site")?;
        let end = text[start..].find("      - name: Run the tests")? + start;
        Some(text[start..end].to_owned())
    };
    let (Some(build), Some(pages)) = (step("build.yml"), step("pages.yml")) else {
        return; // a partial export need not carry every workflow
    };
    assert_eq!(
        build, pages,
        "the site check must be identical in build.yml and pages.yml: one runs on pull requests \
         and the other before publishing, and a check that differs between them lets a pull \
         request pass a site the deploy rejects",
    );
}

/// The read-only build runs on both trunk names, and it has to.
///
/// The repository this app is published to is a fork whose trunk is `main`, so a
/// checkout or pull request there has no `master` branch at all. Listing only
/// `master` would leave every push to that trunk with no build and no site
/// check — green by absence, which is the worst kind of green.
#[test]
fn the_build_workflow_runs_on_both_trunk_names() {
    let Some(build) = live_build_workflow() else {
        return;
    };
    let branches = build
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("branches:"))
        .expect("build.yml must state the branches it runs on");
    for branch in ["master", "main"] {
        assert!(
            branches.contains(branch),
            "build.yml must run on {branch} as well as the other trunk name (found {branches:?}): \
             the fork whose trunk is written above has no `master` at all, and a push there that \
             triggers nothing looks exactly like a passing build",
        );
    }
    assert!(
        build.contains("pull_request:"),
        "build.yml must still run on pull requests: it is the read-only check that a fork's pull \
         request can run",
    );
}

/// The shell carries the four settings controls and the preview checkbox,
/// and the app writes all of them from state on every render.
///
/// The preview setting is this app's honesty rule: with it off, the player's
/// mix colour is the answer to a check and must stay hidden until one runs.
/// The rule lives in `mix.rs` (`reveal_attempt`); this pins the shell half —
/// the checkbox exists, is labelled, and is the only input of its kind.
#[test]
fn the_preview_setting_exists_and_is_labelled() {
    let page = shell();
    assert!(
        page.contains("<input type=\"checkbox\" id=\"show-preview\">"),
        "the preview checkbox is the shell's only checkbox and the reveal rule's control"
    );
    let rust =
        strip_rust_comments(&std::fs::read_to_string(root().join("src/ui.rs")).expect("src/ui.rs"));
    assert!(
        rust.contains("set_checked"),
        "src/ui.rs must drive the checkbox from state, not leave it to the browser"
    );
    // And the rule itself is a tested function of the domain module, not a
    // decision the UI makes from class names.
    let mix = strip_rust_comments(
        &std::fs::read_to_string(root().join("src/mix.rs")).expect("src/mix.rs"),
    );
    assert!(
        mix.contains("pub fn reveal_attempt(&self) -> bool"),
        "the reveal rule must live in the domain module, where tests can hold it"
    );
}

/// The game's every user-visible word is written by the domain module.
///
/// The template's rule — one function owns every word the app shows about the
/// state — is what stops the DOM from becoming a second source of truth. The
/// UI must not compose status strings of its own.
#[test]
fn the_game_words_live_in_the_domain_module() {
    let rust =
        strip_rust_comments(&std::fs::read_to_string(root().join("src/ui.rs")).expect("src/ui.rs"));
    // The UI writes these words, and none of its own: every set_text call
    // takes the domain's own wording.
    for owned in [
        "game.round_line()",
        "game.score_line()",
        "game.status()",
        "game.next_label()",
        "game.round.hint_lines()",
    ] {
        assert!(
            rust.contains(owned),
            "src/ui.rs must render {owned} from the domain module rather than compose text"
        );
    }
}

/// SHA-256 of `bytes`, lowercase hex.
///
/// Self-contained on purpose. Pinning a file's digest is worth a dependency in
/// an app; in a template whose whole argument is that it needs none, it is not
/// — so this is the FIPS 180-4 round constants and 64 rounds, and it is
/// checked against the published test vectors in the test below. If you ever
/// change this function, that test has to keep passing.
// Verbatim FIPS 180-4: the round constants, the 64 rounds and the standard
// a..h state names are the specification; splitting or renaming them would
// only make the code harder to check against it.
#[allow(clippy::many_single_char_names, clippy::too_many_lines)]
fn sha256_hex(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];

    let mut state: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];

    // Pad to a multiple of 64 bytes: 0x80, zeros, then the bit length as u64be.
    let mut message = bytes.to_vec();
    let bits = (bytes.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bits.to_be_bytes());

    for block in message.as_chunks::<64>().0 {
        // The message schedule, in the name the rest of the function uses.
        let mut words = [0u32; 64];
        for (index, word) in block.as_chunks::<4>().0.iter().enumerate() {
            words[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }

        // The eight working variables of FIPS 180-4 §6.2.2, named `v0`..`v7`
        // here because `a`..`h` trip the single-character-name lint; the
        // rotation below is the spec's, only renamed.
        let [mut v0, mut v1, mut v2, mut v3, mut v4, mut v5, mut v6, mut v7] = state;
        for index in 0..64 {
            let s1 = v4.rotate_right(6) ^ v4.rotate_right(11) ^ v4.rotate_right(25);
            let choose = (v4 & v5) ^ (!v4 & v6);
            let temp1 = v7
                .wrapping_add(s1)
                .wrapping_add(choose)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let s0 = v0.rotate_right(2) ^ v0.rotate_right(13) ^ v0.rotate_right(22);
            let majority = (v0 & v1) ^ (v0 & v2) ^ (v1 & v2);
            let temp2 = s0.wrapping_add(majority);

            v7 = v6;
            v6 = v5;
            v5 = v4;
            v4 = v3.wrapping_add(temp1);
            v3 = v2;
            v2 = v1;
            v1 = v0;
            v0 = temp1.wrapping_add(temp2);
        }
        for (slot, value) in state.iter_mut().zip([v0, v1, v2, v3, v4, v5, v6, v7]) {
            *slot = slot.wrapping_add(value);
        }
    }

    let mut hex = String::with_capacity(state.len() * 8);
    for word in state {
        write!(hex, "{word:08x}").expect("writing to String cannot fail");
    }
    hex
}

/// The digest function above is load-bearing for the icon test, so it is
/// checked against the published vectors rather than trusted.
#[test]
fn the_pinned_digest_function_is_correct() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        sha256_hex(b"The quick brown fox jumps over the lazy dog"),
        "d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592"
    );
}

/// `source` with its `//` and `/* ... */` comments dropped.
///
/// Used so an assertion about what `src/ui.rs` *does* cannot be satisfied by a
/// comment saying what it does.
fn strip_rust_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    // A line comment runs to the end of the line, and a block comment to its
    // closer. A `//` inside a string literal is not a comment, but none of the
    // strings these tests look for contain one, and a full Rust tokenizer is
    // not worth the complexity to guard against it.
    while let Some(start) = rest.find('/') {
        let after = &rest[start + 1..];
        if let Some(tail) = after.strip_prefix("//") {
            out.push_str(&rest[..start]);
            rest = tail.split_once('\n').map_or("", |(_, line)| line);
        } else if let Some(tail) = after.strip_prefix("/*") {
            out.push_str(&rest[..start]);
            rest = tail.split_once("*/").map_or("", |(_, line)| line);
        } else {
            out.push_str(&rest[..=start]);
            rest = after;
        }
    }
    out.push_str(rest);
    out
}
