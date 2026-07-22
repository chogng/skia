# Shared shader sources

This directory owns the source-of-truth shader programs for the reusable
`skia` engine. Programs are grouped by rendering capability, not by platform:

- `solid_rect.metal` owns the Metal solid-geometry entry points and the
  atlas-backed glyph vertex/fragment programs. Glyph vertices use target-pixel
  positions and integer atlas coordinates; the fragment program either tints
  Alpha8 masks or preserves RGBA color-glyph samples.
- Future backends add sibling source variants such as `solid_rect.wgsl` while
  preserving the same buffer bindings and output contract.

Platform crates compile these sources into transient build artifacts. Generated
`.air`, `.metallib`, SPIR-V, and pipeline-cache files are never checked in.
