# Owned rendering goldens

These files are generated only from repository-authored scenes in
`gpu/tests/support/render_cases.rs`.  They contain no upstream Skia, font, image, or
other third-party test asset.

`*.rgba` is the authoritative, exact RGBA8 oracle.  The accompanying PNG is
for visual review only.  `manifest.toml` records dimensions, color space, and
SHA-256 digests of both forms.

To intentionally regenerate them, run from the workspace root:

```sh
SKIA_UPDATE_GOLDENS=1 scripts/regenerate_goldens.sh
```

Review the raw-pixel diff, PNG diff, and manifest changes together.  Never
regenerate a golden as part of an ordinary test or CI run.
