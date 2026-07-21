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
  gpu --> metal["Metal backend"]
  path --> tessellation["Shared tessellation"]
  tessellation --> cpu
  tessellation --> metal
  text --> gpu_text["GPU text adapter"]
  gpu_text --> gpu
```

## Dependency rule

- `skia/` (`skia`) is the only public graphics API for consumers.
  `skia/error`, `skia/geometry`, `skia/path`, `skia/tessellation`, `skia/text`,
  `skia/core`, `skia/image`, `skia/codec`, and executor crates are implementation crates;
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
- `skia/tessellation` owns backend-neutral path-to-polyline and path-to-mesh
  algorithms. Its bounded fixed-step curve flattener is shared by CPU and
  hardware backends; backend crates own only their raster or GPU buffer format.
- `skia/gpu` owns only generic GPU resources, atlas quads, commands, and backend
  submission. `skia/gpu/text` is the one-way adapter from font/layout data to
  those primitives. Hardware backends depend on `skia-gpu`, never on the text
  adapter, so adding Vulkan or WebGPU does not duplicate shaping or atlas policy.
- `skia/gpu/metal` and `skia/gpu/vulkan` are platform execution adapters. The
  Vulkan bring-up dynamically loads the platform loader and owns a real instance,
  device, graphics queue, offscreen RGBA8 image, clear submission, and staging
  readback; unsupported draws fail closed without a CPU fallback.
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
can also request underline and strike-through lines globally or per span, with
independently inherited solid, dashed, dotted, or wavy visual patterns.
Their scaled position and thickness come from the selected span's preferred
OpenType face; final visual segments track alignment and justification and stay
continuous across compatible fallback runs. A backend-neutral fixed-point
geometry builder expands every pattern into bounded rectangle strips, which
CPU layout drawing resolves with each segment's style paint after glyph outlines.
Display-list paragraph and layout helpers transactionally expand the same
positioned runs and decoration strips into portable commands, rolling back the
whole expansion if paint resolution, coordinates, or resource budgets fail.
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
language-specific font selection, dictionary data and algorithms, and broader
paragraph formatting remain upper text-layout responsibilities. GPU vector and
atlas text adaptation are available through the separate `skia-gpu-text` adapter.
`layout_outline_batches` converts positioned outlines into ordinary target-space
paths for generic `fill_path` commands. For bitmap text,
`TextAtlasBuilder` rasterizes and packs a `TextLayout`, and `TextAtlas` converts
layout positions into owned generic quads without borrowing an encoder. The
caller then explicitly registers `into_gpu_atlas()` and records the quads with
`skia-gpu`. `layout_decoration_batches` independently converts resolved
underline and strike-through patterns into per-style target-space rectangles;
callers record those through generic `fill_rect` commands. This keeps text data
adaptation separate from command ordering and hardware backends. The Metal
backend draws transformed/scissored solid rectangles, path-fill masks, Alpha8 masks, and color
glyphs through real shader pipelines; rectangle and glyph draws can sample
parent-linked R8 complex-clip masks rendered on the GPU, and these paths
currently support source-over blending. `StrokePath` shares deterministic
normalization, dashing, and cap/join policy with CPU; Metal rasterizes its
fixed-resolution triangle list to R8 before the final blend and clip.
`TextAtlasCache` retains
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
graphics-state operations. Backend-neutral `StrokeOptions` defines
center/inside/outside alignment, butt/round/square caps, miter/round/bevel
joins, miter limits, and canonical dash patterns. Non-center alignment is
defined only for closed, non-degenerate contours and follows contour winding.
CPU Canvas and Metal consume the same expanded triangle mesh; DisplayList and
generic GPU commands preserve the options, and software replay introduces no
backend-specific stroke policy.
Backend-neutral `SamplingOptions` similarly preserves nearest or bilinear
clamp-to-edge image reconstruction through DisplayList and GPU commands. CPU
uses checked affine inverse mapping and deterministic integer bilinear
interpolation; Metal applies the same texel-center convention to arbitrary
affine image draws.
Backend-neutral `ClipOp` defines intersection and difference. CPU Canvas,
DisplayList replay, and the generic GPU encoder apply it to rectangles or paths.
Axis-aligned rectangle intersections retain a scissor fast path; CPU complex
clips use deterministic masks, while generic GPU commands retain immutable
parent-linked `GpuClipId` nodes. The software reference backend replays those
nodes through CPU masks, and Metal materializes only used nodes as transient R8
textures shared by subsequent rectangle, path-fill, and glyph draws in the submission.
CPU fill/stroke/clip and Metal clip-edge generation consume the same bounded,
deterministic fixed-step curve flattener from `skia-tessellation`. Stroke
normalization, dashing, and cap/join/miter coverage also live there; CPU keeps
only device-pixel bounds and raster iteration, while backend-specific mask and
edge formats remain local.
`path_boolean` exposes bounded union, intersection, difference, and XOR over
flattened Q16.16 contours, including holes and self-intersections; empty set
results are represented as `None`, while non-empty output uses non-zero fill.
`trim_path`, `corner_path`, and `discrete_path` provide bounded path effects for
normalized arc-length trimming, deterministic quadratic corner rounding, and
seeded fixed-point contour perturbation. Trim supports wrap-around intervals;
corner radii are clamped to half of each adjacent edge; discrete resampling
keeps open endpoints and closed seams stable. All implement the extensible
`PathEffect` contract and can run left-to-right through `compose_path_effects`
without reapplying the input transform. Tangent-/endpoint-defined arc variants
and additional path effects remain separate work.
`stroke_to_path` is also available through the facade and produces a
deterministic non-zero triangle-fill path.

## Path implementation layout

The public `Path` and `PathBuilder` API is implemented in `skia/path/src/lib.rs`.
Algorithm families are split beneath it so construction contracts do not become
coupled to geometry queries or contour processing:

- `arc.rs` owns public ellipse-arc construction and continuation methods.
- `bounds.rs` owns conservative and polynomial-Bézier extrema bounds helpers.
- `reverse.rs` owns contour parsing and reverse traversal.
- `math.rs` owns checked fixed-point scalar operations shared by path code.

Backend consumers must not add their own Bézier flattening. The reusable
`PathFlattener`, output ceilings, and flattened contour representation live in
`skia/tessellation/src/flatten.rs`, ready for Metal, Vulkan, WebGPU, and CPU
consumers without exposing backend command or buffer types.
