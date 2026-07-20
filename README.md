# Skia subsystem boundary

`skia/` is an independently developed 2D graphics subsystem and reusable
library. It owns portable geometry, paths, paints, image resources and codecs,
text-glyph drawing contracts, display lists, and CPU/GPU execution. It is **not** an
implementation detail of a particular caller and it does not model caller-specific
operators or objects.

```mermaid
flowchart LR
  adapter["Upstream adapter"] --> api["Skia public API\nskia facade"]
  other["Other library adapters"] --> api
  api --> geometry["Geometry"]
  api --> path["Path"]
  api --> text["Font loading and shaping"]
  api --> image["Image resources"]
  api --> codec["Image codecs"]
  api --> core["Skia core semantics"]
  api --> cpu["Skia CPU executor"]
  api --> gpu["Skia GPU executor"]
  text --> gpu_text["GPU text adapter"]
  gpu_text --> gpu
```

## Dependency rule

- `skia/` (`skia`) is the only public graphics API for consumers.
  `skia/error`, `skia/geometry`, `skia/path`, `skia/text`, `skia/core`, `skia/image`,
  `skia/codec`, and executor crates are implementation crates;
  consumers must not depend on them directly. Skia crates may depend on each
  other, but never on a caller-specific document crate or semantic type.
- The facade exports an explicit, stable set of canvas, geometry, paint, path,
  image, text-outline, and error types. It does not expose display-list
  resource IDs, command representations, or backend command encoders.
- `skia/error` contains shared failure types; `skia/geometry` contains fixed
  point coordinates and affine transforms; `skia/path` contains immutable
  paths and path construction. Their dependencies flow only downward.
- `skia/core` contains paint and backend-neutral display-list semantics. It
  depends on the foundational crates but never on an executor, platform
  graphics API, caller-specific parser, document model, or Scene. Its default
  `text` feature adds glyph-run display-list resources; GPU crates disable that
  feature because generic atlas submission does not need shaping types.
- `skia/gpu` owns only generic GPU resources, atlas quads, commands, and backend
  submission. `skia/gpu/text` is the one-way adapter from font/layout data to
  those primitives. Hardware backends depend on `skia-gpu`, never on the text
  adapter, so adding Vulkan or WebGPU does not duplicate shaping or atlas policy.
- `skia/image` owns the immutable RGBA8 resource representation. `skia/codec`
  parses untrusted, general-purpose image bytes into that representation and
  encodes those resources as general-purpose image formats. It does not depend
  on rendering backends or caller-specific types, so both decode and encode
  remain in `skia/codec`, not in the resource crate.
- Every consumer calls Skia only through its public API. Each consumer owns its
  source-domain adapter and reports its rendering
  intent, target description, and source data to the Skia upper integration
  layer. That layer owns resource lifetime and executor selection before
  calling lower Skia components.
- A Skia public type, method, error, or command must not mention caller-specific
  objects, operators, page state, or policy. Perform such translation in the
  caller's adapter.

## Text implementation boundary

`skia/text` owns portable font identities, ordered in-memory font collections,
shaped glyph runs, source UTF-8 clusters, bidi visual runs, and validated
vector outlines. These remain one cohesive crate: its root only assembles and
re-exports the public API, while internal modules separate foundational glyph
types, outline contracts, font processing, collections, and layout. This is a
source-organization boundary, not a new dependency or runtime boundary.
`FontFace` owns TrueType/OpenType data and provides
segment-level shaping plus outline resolution. A face also exposes its preferred
OpenType family name, normalized weight/width/slant, and variable-font axes.
Validated Q16.16 axis coordinates create immutable instances with distinct
`FontId` values, and consistently affect shaping, metrics, and outlines.
Immutable feature instances also apply global OpenType values such as `kern=0`
through every single-run, fallback, bidi, and multiline shaping path.
BCP 47-style language tags can likewise be supplied to face, paragraph,
styled, and multiline APIs so language-sensitive OpenType substitutions such
as `locl` remain consistent through fallback, bidi segmentation, wrapping,
hyphenation, and ellipses.
`FontCollection` provides deterministic CSS-like family/style matching,
performs grapheme-level ordered fallback, and shapes unwrapped or greedily
wrapped bidi text into positioned visual runs. Styled spans can select a
preferred immutable face instance and Q16.16 size per grapheme-safe source
range across line boundaries, while retaining fallback and bidi behavior.
They also preserve a renderer-neutral `TextStyleId` and optional decoration
override, allowing CPU and GPU adapters to resolve per-span paints without a
dependency from text layout back to paint semantics.
Every wrap candidate is reshaped independently, and empty hard-break lines use
the logical line-start style's metrics. Layout work remains explicitly bounded.
CPU drawing reuses the ordinary path-fill pipeline. Laid-out lines carry
physical left/center/right alignment or bidi-aware logical start/end alignment.
Justified lines preserve shaping output
and add deterministic per-glyph spacing at interior breakable Unicode spaces,
including ideographic space while excluding non-breaking spaces. If no such
space exists, automatic mixed CJK/script boundaries or an explicit
cross-script inter-character policy distribute width without splitting marks,
ligatures, whitespace, controls, or punctuation.
Callers can also add signed Q16.16 letter spacing between shaping clusters and
word spacing after breakable Unicode spaces; wrapping, ellipses, hit testing,
and carets all use the resulting width without splitting grapheme or shaping clusters.
Callers can plug language dictionaries into `TextBreakProvider`; the layout
engine validates UTF-8 grapheme boundaries and supports either glyph-free soft breaks
or synthetic visible hyphens without consuming source bytes. Layout options
can also request underline and strike-through lines globally or per span.
Their scaled position and thickness come from the selected span's preferred
OpenType face; final visual segments track alignment and justification and stay
continuous across compatible fallback runs. CPU layout drawing resolves each
segment's style paint after glyph outlines.
`TextLayout` also maps layout-local points to editable UTF-8 boundaries and
resolves source positions back to vertical carets. Font-provided OpenType GDEF
ligature caret coordinates add internal stops without dividing shaping output.
Upstream/downstream affinity
distinguishes soft-wrap and bidi boundary positions; alignment, justification,
synthetic hyphens, empty lines, and mixed line metrics are included.
Caret-boundary source ranges resolve to line-local `TextSelectionRect`
geometry, including partial ligature components when GDEF data is available.
Wrapped ranges split by line, bidi ranges split by visual
discontinuity, and synthetic markers remain excluded.
Line limits default to an all-or-error resource policy. Callers can explicitly
select clipped output or a grapheme-safe, reshaped final-line ellipsis.
Ellipses retain styled font size and bidi placement, prefer U+2026, and fall
back to three periods without consuming source bytes.

System-font discovery, generic-family mapping, variable-font instance selection,
language-specific font selection, dictionary data and algorithms, and
decorative line variants remain upper text-layout responsibilities. GPU glyph
atlases are available through the separate `skia-gpu-text` adapter:
`TextAtlasBuilder` rasterizes and packs a `TextLayout`, and `TextAtlas` converts
layout positions into owned generic quads without borrowing an encoder. The
caller then explicitly registers `into_gpu_atlas()` and records the quads with
`skia-gpu`, keeping data adaptation separate from command ordering. The Metal
backend draws Alpha8 masks and color glyphs through a real shader pipeline; it
currently supports source-over blending for this path. `TextAtlasCache` retains
bounded immutable packed atlases with least-recently-used eviction, while stable
generic atlas keys let Metal retain and reuse a separately bounded native
texture across submissions. Both layers expose hit, upload, and eviction stats;
font identities and requested raster sizes remain the caller's invalidation
boundary.

## Geometry and transforms

Paths are immutable geometry resources. `PathBuilder` constructs paths from
generic 2D primitives; it must not encode caller-specific path or graphics-state rules.
Canvas and display-list transforms are generic affine drawing state that apply
to subsequent drawing operations. A consumer
that has a source-specific matrix is responsible for mapping it at its adapter
boundary.

Current primitive construction includes rectangles, circles, ellipses, rounded
rectangles, polygons, deterministic cardinal arcs, arbitrary-angle and rotated
ellipse arcs up to one full turn,
quadratic and rational-quadratic Béziers, and cubic Béziers. Paths can be
transformed, appended, reversed, and queried for both conservative
control-point bounds and curve-extrema-aware conservative bounds (with rational
quadratics retaining their control hull). `DisplayList` and the GPU encoder
expose both transform replacement and affine concatenation as generic
graphics-state operations. Boolean path operations, stroke-to-path expansion,
path effects, and tangent-/endpoint-defined arc variants remain separate
geometry-processing work; their design must stay independent of any consumer.

## Path implementation layout

The public `Path` and `PathBuilder` API is implemented in `skia/path/src/lib.rs`.
Algorithm families are split beneath it so construction contracts do not become
coupled to geometry queries or contour processing:

- `arc.rs` owns public ellipse-arc construction and continuation methods.
- `bounds.rs` owns conservative and polynomial-Bézier extrema bounds helpers.
- `reverse.rs` owns contour parsing and reverse traversal.
- `math.rs` owns checked fixed-point scalar operations shared by path code.
