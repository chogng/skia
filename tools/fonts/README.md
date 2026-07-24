# Font test tooling

This directory owns the repository-authored tooling and frozen data for font
tests. It is the Rust counterpart of upstream Skia's `tools/fonts/`:
generators, the portable TestTypeface model, and their format-specific
validation belong here rather than in an individual test crate or `scripts/`.

`generate_test_fonts.rs` deterministically creates the small SFNT/TTC inputs
used by the Rust text tests. Its checked-in outputs live in
`skia-rs/text/tests/fonts/synthetic/`, alongside the tests that consume them.
Keeping the outputs there makes Cargo and Bazel test inputs explicit; keeping
the generator here gives all crates one maintenance entry point.

`test_typeface.rs` is a direct Rust adaptation of upstream Skia's checked-in
TestTypeface data. It contains the unchanged logical dataset for all twelve
Liberation Mono, Sans, and Serif faces: glyph outlines, path verbs, character
maps, fixed advances, metrics, and style metadata. `skia-text` imports this
test-only module as an internal Typeface backend and exercises it through the
production `FontCollection` matching, shaping, metrics, and outline paths. It
does not replace the real SFNT/TTC fixtures or OpenType shaping coverage.

The data is deliberately committed as Rust constants. Do not add a runtime or
build-time conversion step: upstream's original generator reads host font
installations and is unsuitable as a portable test prerequisite. See
`UPSTREAM_TEST_TYPEFACE.toml` for source hashes, licensing, and the exact Skia
revision. There is intentionally no checked-in converter.

The pinned upstream fonts and their license/provenance metadata are not owned
by this tool. They remain under `skia-rs/text/tests/fonts/skia/`. Larger
third-party corpora remain manifest-driven under `skia-rs/text/tests/fonts/extra/`.

Regenerate the synthetic fixtures from the repository root:

```text
rustc --edition 2024 tools/fonts/generate_test_fonts.rs -o skia-rs/target/generate-test-fonts.exe
skia-rs/target/generate-test-fonts.exe skia-rs/text/tests/fonts/synthetic
```
