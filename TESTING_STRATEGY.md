# Skia subsystem testing strategy

## Scope and current baseline

This repository is an independently implemented Rust 2D subsystem, not a
binding to Google Skia.  Google Skia's test programs, ABIs, serialization, and
golden-image keys are therefore *evidence and input sources*, not an oracle
that can be dropped into this workspace unchanged.

The current baseline is useful but predominantly behavioural: workspace Rust
tests cover the public facade, CPU canvas, display lists, paths/path effects,
the software GPU replay contract, Metal, Vulkan offscreen execution, codecs, and text.
Run Rust commands from `skia-rs/`; `cargo test --workspace --exclude skia-metal --exclude skia-vulkan
--all-features` is the portable regression gate; platform executors run in
their dedicated Metal and forced-Lavapipe jobs.  The three full Unicode
conformance tests are intentionally external, checksum pinned downloads; run
`scripts/fetch_unicode_conformance.sh skia-rs/target/unicode-conformance` from
the repository root, followed by the command in
`skia-rs/text/tests/data/unicode/SOURCES.md`.

Known gaps are a checked-in/rendered pixel-golden harness, a versioned media
fixture manifest, cross-backend scene comparison, property/fuzz targets,
sanitizer jobs, a required-Metal CI pool, and an automated license/provenance
gate for binary test assets.  System-font discovery tests also necessarily
exercise the host and are not a portable text-layout oracle.

## What upstream Skia uses

Google Skia's current primary correctness tool is DM.  It runs C++ unit tests
from `tests/`, rendering GMs from `gm/`, image inputs from `resources`, and
optionally SKP recordings, against a source/sink configuration matrix.  Its
documentation describes the standard CPU `8888` and GPU configurations,
parallel execution, emitted raw-pixel checksums, and local image comparisons:

- [Skia correctness testing / DM](https://skia.org/docs/dev/testing/testing/)
- [current `tests/` tree](https://skia.googlesource.com/skia/+/main/tests/)
- [current `gm/` tree](https://skia.googlesource.com/skia/+/main/gm/)
- [writing unit and rendering tests](https://skia.org/docs/dev/testing/tests/)

Its production-scale image comparison service is Gold.  Baselines are outside
Git and are triaged per platform/backend; Skia reports more than 500,000
images per commit.  Gold therefore is a service model, not a directory of
portable expected PNGs.  See [Skia Gold](https://skia.org/docs/dev/testing/skiagold/).

Skia also fuzzes its C++ implementation locally and through OSS-Fuzz, with
special fuzz build defines and fuzzer-specific harnesses.  The upstream guide
points to its in-tree `fuzz/` code and `oss-fuzz/projects/skia`; OSS-Fuzz warns
that corpus seeds must have a consistent suitable license.  See [Skia
fuzzing](https://skia.org/docs/dev/testing/fuzz/) and the [OSS-Fuzz new-project
guide](https://google.github.io/oss-fuzz/getting-started/new-project-guide/).

CanvasKit is a distinct JS/WASM surface.  Its Jasmine/Karma `tests/` include
its own small assets and can submit canvas snapshots to Gold; Puppeteer is used
for its browser performance harness.  It is relevant only if this repository
later exports a compatible WASM/web API.  See [CanvasKit's current README and
test tree](https://skia.googlesource.com/skia/+/refs/heads/main/modules/canvaskit)
and [its Puppeteer performance harness](https://skia.googlesource.com/skia/+/refs/heads/main/tools/perf-canvaskit-puppeteer/).

## Adoption matrix

| Upstream/standard asset | Classification | Rust adoption decision | License and provenance rule |
| --- | --- | --- | --- |
| `tests/` C++ unit tests, especially pathops/codec/color tests | Test code | **Adapter/translation only.** Recreate the invariant through the Rust public API; never compile or include C++ harness internals. | Skia source is BSD-3-Clause, but retain copyright/license and link to the exact source when translating. Skia's top-level license also calls out exceptions. |
| `gm/` C++ render scenes | Test code / scene ideas | **Adapter/translation only.** Select a small Rust scene vocabulary and render it through CPU, software GPU, and Metal. | Same BSD-3-Clause source condition; do not import a GM plus its transitive resources without individual provenance. |
| DM source/sink matrix and raw-pixel checksums | Test infrastructure design | **Directly reuse the design, not code.** Our equivalent is a Rust scene manifest plus backend runners. | No data copied. |
| Gold baselines, digests, and triage results | Golden images/metadata | **Do not copy.** Generate repository-owned, reproducible baselines only after an explicit review workflow exists. | Gold baseline content is external to Git and configuration-specific. It is neither a stable API nor a redistribution-ready corpus. |
| `resources/`, CanvasKit assets, fonts, images, SVGs | Binary third-party data | **Do not bulk copy.** Admit an individual item only through a manifest recording URL/revision, SHA-256, SPDX/license text, purpose, and size. | Skia's own `LICENSE` explicitly contains resource exceptions (including Openclipart/public-domain material); the tree is not uniformly BSD-3-Clause. |
| downloaded SKPs / CIPD assets | Large recordings/data | **Do not vendor; CI-download only if a future SKP reader exists.** Current Rust API neither reads SKP nor shares Skia serialization. | Upstream ignores local `skps/`; retain upstream package provenance and do not assume every recording has a redistribution grant. |
| `fuzz/` and OSS-Fuzz harnesses | Test code | **Design reference / adapter.** Add Rust `cargo-fuzz` targets against this crate's decode, path, display-list, and text boundaries. | Do not treat an OSS-Fuzz public corpus URL as a versioned, licensed vendoring source; keep only locally minimized, provenance-reviewed reproducers. |
| CanvasKit Jasmine/Karma/Puppeteer tests | JS/WASM test code | **Design reference only** until a Rust WASM/CanvasKit API is introduced. | The API, build toolchain, and browser image semantics differ. |
| Unicode UCD conformance files | Input corpus | **Direct CI download; already implemented.** | Unicode Data Files are under Unicode License v3 unless individually noted; URLs and SHA-256 values remain in the fetch script and data stays under `target/`. [Unicode terms](https://www.unicode.org/copyright.html). |
| ICC/color profiles, PDF/image comparison inputs, SVG suites | External corpora | **Not yet admitted.** Use generated color cases and API-level properties first; add third-party files only through the same manifest. | Profiles, fonts, and documents have independent licences. This subsystem currently has no PDF or SVG API, so those suites would not test a supported boundary. |

The source-license statement above is based on [Skia's current
LICENSE](https://skia.googlesource.com/skia/+/main/LICENSE), rather than an
assumption that every path in the repository has the same terms.

## Phased implementation

### Phase 0 — portable gate (now)

Run formatting, Clippy with warnings denied, the portable workspace tests
excluding Metal and Vulkan executors, and the checksum-verified Unicode
conformance suite on Linux.
The Unicode gate tests grapheme boundaries, line breaks, and bidi ordering
against the exact data version advertised by each Rust dependency.  It is
small enough to download in CI (roughly 8 MiB) and avoids vendoring any binary
asset.

On macOS, run the ordinary Metal tests regardless of device availability.  A
separate, protected required-hardware job must run on a known device and set
`SKIA_REQUIRE_METAL_DEVICE=1`; it must not be silently replaced by an
emulated/absent-device runner.  The ordinary job protects compilation and
device-less behaviour, while the protected job protects pixel execution.

On Linux, the required `vulkan-lavapipe` job selects Mesa's software Vulkan ICD,
enables `VK_LAYER_KHRONOS_validation`, and sets `SKIA_REQUIRE_VULKAN_DEVICE=1`.
This makes loader, synchronization, staging upload/readback, and portable-command
pixel tests mandatory on every change without relying on a hosted runner having
a physical GPU.  Vendor hardware remains a separate main/nightly runner concern.

### Phase 1 — owned scenes and pixel oracle (initial implementation complete)

`skia-rs/gpu/tests/support/render_cases.rs` now contains four repository-authored scenes that
exercise clips/alpha, even-odd paths/transforms, layers/gradients, and linear
image sampling.  `skia-rs/gpu/tests/render_oracle.rs` renders them through CPU and
`skia-gpu` software replay, requiring exact RGBA8 equality and dimensions.
`skia-rs/tests/golden/` holds their reviewed raw-pixel/PNG fixtures and manifest.

Introduce `skia-rs/tests/golden/manifest.toml` only for explicitly accepted expected
images.  Each entry should include renderer version, scene ID, width/height,
pixel format/color space, SHA-256 of raw RGBA and PNG, and an update reason.
`scripts/regenerate_goldens.sh` refuses to overwrite without
`SKIA_UPDATE_GOLDENS=1`; it records SHA-256 for raw RGBA and PNG output.
Never use PNG-file checksums alone.  The next increment should add fixed
locale/timezone and a JSON diff summary before the scene set grows materially.

### Phase 2 — backend matrix and tolerances

Run every owned scene against CPU, software GPU, and Metal.  CPU/software is
bit exact.  Metal has two classes: exact tests for integer/blend/clip scenes,
and explicitly named tolerant tests for sampling/filtering.  Tolerances belong
in the manifest per channel and per pixel-count (default zero); report maximum
absolute error, differing-pixel count, and an amplified diff PNG.  No global
"close enough" threshold.

CI lanes:

| Lane | Trigger | Required work |
| --- | --- | --- |
| Linux portable | every change | fmt, Clippy, workspace tests, Unicode download/conformance |
| Linux Vulkan Lavapipe | every change | forced software ICD, validation layer, Vulkan command/readback tests |
| macOS device-optional | every change | workspace/Metal tests; absence is visible but allowed |
| macOS Metal-required | protected runner, main/nightly and GPU changes | `SKIA_REQUIRE_METAL_DEVICE=1`, selected scene comparison, artifact upload on mismatch |
| fuzz/property | nightly and changed boundary | bounded fuzz smoke plus deterministic property seeds |
| corpus refresh | manual, review-only | re-download manifest, verify hashes/licenses; no automatic baseline update |

### Phase 3 — semantic and hostile-input testing

For paths/pathops/strokes/effects, add generated bounded inputs and properties:
deterministic output, transform/inverse identities where defined, containment
relations for boolean results, no panics, output-limit failure, and CPU versus
software-replay equivalence.  Translate selected upstream PathOps failure
shapes one-by-one rather than importing the C++ data tables.

For text, retain the Unicode suite and add controlled, licensed test fonts
only after their licence is recorded.  Separate deterministic embedded-font
tests from host system-font discovery.  Cover shaping/fallback/bidi/layout,
language tags, dictionary boundaries, variable axes, and color/bitmap glyphs
with known fonts; never make system-font glyph pixels a portable golden.

For PNG/JPEG/WebP, keep the current `codec/src/api/png.rs` work untouched.  Add
decode limits, metadata, malformed/truncated input, and encode/decode
properties at the public `ImageCodec` boundary.  A future media corpus belongs
in a downloaded manifest and needs individual source licences; it must not be
copied wholesale from Skia `resources`, OSS-Fuzz, or CanvasKit assets.

Add `cargo-fuzz` targets under `fuzz/` for PNG/JPEG/WebP decode, path parsing
and boolean operations, display-list replay, and font parsing/layout.  Run
short deterministic `-runs=` smoke jobs in CI, sanitize on nightly runners,
and commit only minimized crashing inputs with a source/reproducer and licence
record.

## Admission policy for any future test asset

Before a binary, corpus, font, profile, or golden enters this repository,
record its exact upstream URL/revision, SHA-256, byte size, licence/SPDX or
verbatim notice location, owner, tested API, update command, and whether it is
vendored or CI-downloaded.  Reject it if any of those fields is unknown.  This
is particularly important for fonts, images, SVGs, SKPs, and fuzz inputs:
content licence and code licence are independent.

The first admissible external corpus remains the existing Unicode data: it is
authoritative, compact, licence-identified, hash-pinned, and maps directly to
the Rust text APIs.  Everything else above requires an adapter, an individual
licence review, or should remain only a design reference.
