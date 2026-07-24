# Test fonts

This directory owns the font inputs used by `skia-text` tests and by downstream
Rust tests in `skia-core`, `skia-cpu`, and `skia-pdf`. Sharing font files from
this directory is intentional. Sharing Rust test modules across crates with
`#[path]` is not.

The font inputs follow the three models used by upstream Skia:

```text
fonts/
|-- synthetic/
|-- skia/
`-- extra/
```

The non-SFNT upstream TestTypeface dataset is maintained separately in
`tools/fonts/test_typeface.rs`: it is direct Rust data compiled into
`skia-text`'s unit tests as an internal Typeface backend. Those tests exercise
the production `FontCollection` matching, shaping, metrics, and outline paths.
It is not a replacement for the font-file fixtures below.

## `synthetic`

`synthetic/` contains small, repository-authored fonts with deliberately
controlled character coverage, metrics, outlines, and OpenType tables. They
are the Rust equivalent of upstream Skia's portable test typefaces.

The reviewable source is `tools/fonts/generate_test_fonts.rs`. Generation happens
offline; tests must not construct a complete SFNT font at runtime. Commit the
generated TTF or TTC files so the portable test suite does not depend on
font-generation tools. Regenerate them from the repository root with:

```text
rustc --edition 2024 tools/fonts/generate_test_fonts.rs -o skia-rs/target/generate-test-fonts.exe
skia-rs/target/generate-test-fonts.exe skia-rs/text/tests/fonts/synthetic
```

Use these fonts for deterministic layout, fallback, rendering, subsetting, and
table-boundary tests. A test may copy a fixture and make a small, explicit byte
mutation when malformed input is the behavior under test.

## `skia`

`skia/` contains the selected real font fixtures used by upstream Skia under
`resources/fonts`. Preserve that relative path below this directory so a Rust
test can be traced back to its upstream counterpart:

```text
skia/
|-- UPSTREAM.toml
|-- METADATA.toml
|-- LICENSES/
`-- resources/
    `-- fonts/
```

`UPSTREAM.toml` must pin the Skia repository URL and revision.
`METADATA.toml` must record, for every admitted font, its upstream path,
SHA-256 digest, byte size, purpose, and applicable license or notice. Do not
copy the upstream font directory wholesale: add only files exercised by the
Rust test suite and review each file's provenance independently.

These fixtures are checked into the repository and form part of the portable
test gate. Use them for real TTF, OTF, TTC, variable-font, color-font, and PDF
compatibility coverage.

## `extra`

`extra/` describes the larger real-font corpus injected into upstream
SkParagraph tests, such as Roboto, Noto, Arabic, emoji, and CJK families. Do
not commit those font binaries here. Commit only a manifest containing pinned
download URLs, versions, SHA-256 digests, sizes, and license information.

Once the manifest contains individually pinned font entries, its fetch command
installs verified files under:

```text
skia-rs/target/extra-fonts/
```

CI jobs that enable extended paragraph and shaping coverage must fetch and
require this corpus. Local tests may skip that coverage when the corpus is
absent, but must report the configured fetch command rather than silently
substituting host system fonts. Until the manifest has complete entries, no
Rust test may claim `extra/` coverage.

## Usage rules

- Prefer `include_bytes!` for checked-in fixtures so tests are independent of
  the process working directory.
- Bazel targets must declare every referenced font as an input.
- Keep host system-font discovery separate from deterministic font behavior;
  host glyphs and metrics are not portable expected values.
- A test may use more than one category. The directories describe provenance
  and lifecycle, not exclusive test suites.
- New external fonts require complete source and license metadata before they
  enter the repository.
