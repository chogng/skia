# Font test tooling

This directory owns the repository-authored tooling for synthetic font fixtures.
It is the Rust counterpart of upstream Skia's `tools/fonts/`: generators and
their format-specific validation belong here, rather than in an individual
test crate or under `scripts/`.

`generate_test_fonts.rs` deterministically creates the small SFNT/TTC inputs
used by the Rust text tests. Its checked-in outputs live in
`skia-rs/text/tests/fonts/synthetic/`, alongside the tests that consume them.
Keeping the outputs there makes Cargo and Bazel test inputs explicit; keeping
the generator here gives all crates one maintenance entry point.

The pinned upstream fonts and their license/provenance metadata are not owned
by this tool. They remain under `skia-rs/text/tests/fonts/skia/`. Larger
third-party corpora remain manifest-driven under `skia-rs/text/tests/fonts/extra/`.

Regenerate the synthetic fixtures from the repository root:

```text
rustc --edition 2024 tools/fonts/generate_test_fonts.rs -o skia-rs/target/generate-test-fonts.exe
skia-rs/target/generate-test-fonts.exe skia-rs/text/tests/fonts/synthetic
```
