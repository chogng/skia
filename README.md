# Skia subsystem boundary

`skia-rs/` is the Rust workspace for an independently developed 2D graphics subsystem and reusable
library. It owns portable geometry, paths, paints, image resources and codecs,
text-glyph drawing contracts, display lists, and CPU/GPU execution. It is **not** an
implementation detail of a particular caller and it does not model caller-specific
operators or objects.

Cargo remains the dependency and package source of truth. The repository also
contains an initial Bazel build rooted at `MODULE.bazel`: `rules_rs` reads
`skia-rs/Cargo.toml` and `skia-rs/Cargo.lock`, while each crate has a small
`BUILD.bazel` target for library and test ownership. Use `bazel build //skia-rs/...`,
`bazel test //skia-rs/...`, or `bazel build --config=clippy //skia-rs/...`.
The existing Cargo workflow remains authoritative while Bazel coverage is being
introduced and validated on every supported platform. Native Windows Bazel builds
require the MSVC C++ build tools used by `rules_rust`; set `BAZEL_SH` to Git Bash
when analyzing or running Rust test targets.

```mermaid
flowchart LR
  adapter["Upstream adapter"] --> api["Selected public crate APIs"]
  other["Other library adapters"] --> api
  api --> geometry["Geometry"]
  api --> path["Path"]
  api --> text["Font loading and shaping"]
  api --> image["Image resources"]
  api --> codec["Image codecs"]
  api --> core["Skia core semantics"]
  api --> effects["Built-in effects"]
  api --> cpu["Skia CPU executor"]
  api --> gpu["Skia GPU executor"]
  gpu --> metal["Metal backend"]
  gpu --> vulkan["Vulkan backend"]
  path --> tessellation["Shared tessellation"]
  core --> effects
  tessellation --> effects
  tessellation --> cpu
  tessellation --> metal
  tessellation --> vulkan
  text --> gpu_text["GPU text adapter"]
  gpu_text --> gpu
```

## Platform support

Common library targets have no Bazel operating-system constraint and build on
Windows, Linux, and macOS. Backend targets describe only their real native
availability; there is no separate `portable` platform, crate, or feature.

| Capability | Windows | Linux | macOS |
| --- | --- | --- | --- |
| Portable crates, CPU, text, codecs, and shared GPU contracts | Yes | Yes | Yes |
| Vulkan backend | Yes | Yes | No |
| Metal backend | No | No | Yes |

Vulkan on macOS is not part of the supported matrix until MoltenVK loading,
packaging, and CI coverage are explicitly provided. Cargo selects backend crates
at the application composition boundary; Bazel expresses the same boundary with
`target_compatible_with` on the Metal and Vulkan targets.

## Dependency rule

- `skia-rs/Cargo.toml` is a virtual workspace manifest, not a package. There is
  no root facade crate: consumers depend directly on the public responsibility
  crates they use, such as `skia-core`, `skia-effects`, `skia-cpu`,
  `skia-codec`, or `skia-gpu`. Skia crates may depend on each other, but never
  on a caller-specific document crate or semantic type.
- The application composition root may additionally depend on `skia-rs/gpu`,
  `skia-rs/gpu/text`, and one selected platform executor such as `skia-rs/gpu/metal` or
  `skia-rs/gpu/vulkan`. These crates form the public renderer-integration SPI: they own device
  setup, resource lifetime, backend selection, and submission, but are not the drawing API used
  by ordinary domain or rendering code.
- Each crate exposes only the contracts owned by its responsibility. Shared
  display-list semantics live in `skia-core`; CPU `Canvas` and `Surface` live
  in `skia-cpu`; GPU commands and platform devices remain in their GPU crates.
- `skia-rs/error` contains shared failure types; `skia-rs/geometry` contains fixed
  point coordinates and affine transforms; `skia-rs/path` contains immutable
  paths and path construction. Their dependencies flow only downward.
- `skia-rs/core` contains paint and backend-neutral display-list semantics. It
  depends on the foundational crates but never on an executor, platform
  graphics API, caller-specific parser, document model, or Scene. Its default
  `text` feature adds glyph-run display-list resources; GPU crates disable that
  feature because generic atlas submission does not need shaping types.
- `skia-rs/effects` contains the built-in effect implementation and factory
  surface. `skia-core` owns stable effect value and extension contracts so it
  never depends on concrete effects; `skia-effects` depends one-way on core and
  shared tessellation. It currently provides dash, trim, corner, discrete,
  compose, and sum path effects together with built-in gradient, color-filter,
  and image-filter factories.
- `skia-rs/tessellation` owns backend-neutral path-to-polyline and path-to-mesh
  algorithms. Its bounded fixed-step curve flattener is shared by CPU and
  hardware backends; backend crates own only their raster or GPU buffer format.
- `skia-rs/gpu` owns only generic GPU resources, atlas quads, commands, surface
  formats, device capabilities, limits, and backend submission. `skia-rs/gpu/text`
  is the one-way adapter from font/layout data to GPU atlases and glyph quads;
  portable outline and decoration geometry remains in `skia-core`. Hardware
  backends depend on `skia-gpu`, never on the text adapter, so adding Vulkan or
  WebGPU does not duplicate shaping or atlas policy.
- `skia-rs/gpu/metal` and `skia-rs/gpu/vulkan` are platform execution adapters. The
  Vulkan adapter dynamically loads the platform loader and owns a real instance,
  device, graphics queue, compute pipeline, and offscreen RGBA8 storage target.
  It executes the complete portable command vocabulary in Vulkan shaders,
  preserves target contents across submissions, expands path/stroke geometry on
  the host, and reads pixels back from device-owned memory. The CPU renderer is
  used only as a test oracle and is not a production dependency of this adapter.
- `skia-rs/text/system` is the platform filesystem adapter for system/user font
  discovery, generic-family resolution, and language-preferred family policy.
  It returns stable path/index identities and reloadable records; `skia-rs/text`
  remains independent of operating-system directories and font handles.
- `skia-rs/image` owns immutable pixel storage, row layout, alpha semantics,
  color spaces, and bounded pixel/color conversion. `skia-rs/codec`
  parses untrusted, general-purpose image bytes into that representation and
  encodes those resources as general-purpose image formats. It does not depend
  on rendering backends or caller-specific types, so both decode and encode
  remain in `skia-rs/codec`, not in the resource crate.
- Each consumer owns its source-domain adapter and depends on only the Skia
  responsibility crates required by that adapter. Its composition root owns
  resource lifetime and executor selection and may use the renderer SPI
  directly.
- A Skia public type, method, error, or command must not mention caller-specific
  objects, operators, page state, or policy. Perform such translation in the
  caller's adapter.

## Image pixels and color management

`skia-image` is the foundational, backend-independent image layer. `ImageInfo`
describes dimensions, `PixelFormat`, `AlphaType`, and `ColorSpace`; `Image`
separately owns an explicit row stride and exact storage. RGBA8888 and
BGRA8888, including padded rows, have implemented read and conversion paths.
Straight, premultiplied, and opaque alpha are validated at construction.
Premultiplied RGB must not exceed alpha, opaque storage must contain alpha 255,
and a zero-alpha premultiplied pixel converts to transparent black.

Color conversion reuses the pure-Rust `moxcms` dependency already present in
the codec stack, but `skia-image` gates profiles to its bounded RGB matrix/TRC
path. Built-in sRGB and linear sRGB are supported. Embedded ICC is accepted
only when it parses as RGB/XYZ with three tone curves, contains no AToB/BToA
LUT, and can build a transform to sRGB; malformed profiles, CMYK, device-link,
abstract/named-color, and LUT profiles fail explicitly and are never
interpreted as sRGB. Original
accepted ICC bytes are retained for re-encoding. Linear sRGB is serialized as
an ICC profile when a codec cannot otherwise carry that interpretation.

Codecs decode to tight straight RGBA8888 while preserving the accepted source
color space. Display-list registration, direct CPU image drawing, and generic
GPU image registration are the rendering-resource boundaries: each converts
exactly once to tight straight sRGB RGBA8888. Consequently CPU sampling,
software replay, Metal texture upload, and Vulkan immutable-resource upload
receive the same byte order, alpha representation, and working color space.
Color transforms operate on straight RGB; premultiplication is removed before
conversion and applied only after conversion when requested.

Current support is deliberately limited to interleaved eight-bit RGBA/BGRA.
There is no RGB10, RGBA16F, planar YUV, HDR transfer function, CMYK rendering,
arbitrary ICC CLUT, or renderer-selected wide-gamut target yet. Render-target
storage and readback remain straight sRGBA8888, but every CPU, software-GPU,
Metal, and Vulkan compositing operation decodes RGB to a bounded eight-bit
linear-light working representation, performs premultiplied-alpha blending,
and encodes RGB back to sRGB; alpha remains linear. This applies to all blend
modes, layer restore, glyph masks, and blend color filters. Color-managed
images are converted before sampling, so samples from different declared
spaces enter that same compositing path instead of being mixed as if they
shared an encoding.

## PDF output

`skia-pdf::PdfDocument` uses an explicit `begin_page` /
`add_display_list` / `end_page` lifecycle, plus an ergonomic `add_page`
operation. `finish` and `abort` consume the writer, preventing repeated
closure. An unfinished or nested page, unbalanced save state, invalid page
geometry, output failure, and every configured resource ceiling produce a
stable `PdfErrorCode`; unsupported commands are never discarded.
Serialization is delayed until `finish`, so command mapping is transactional
and an error does not emit a misleading partial PDF. The destination can
still fail during the final write, in which case the I/O category is retained.

`PdfOptions::color_policy` defaults to `NativePdf`, preserving vectors through
the standard PDF transparency and blend-mode operators, as SkPDF does. Select
`LinearMatch` to retain this renderer's linear-light compositing contract:
pages containing translucent paint or images, a non-`SourceOver` blend mode,
a translucent clear, or a saved layer are routed through the existing CPU
whole-page fallback. With `UnsupportedBehavior::Error`, those pages fail
explicitly instead. Opaque `SourceOver` vector drawing remains native in either
mode. This makes the vector-versus-pixel-fidelity trade-off explicit rather
than relying on a PDF viewer's blend color space.

The crate deliberately uses a small in-tree PDF writer rather than a general
PDF object-model dependency. The required surface is narrow (new documents,
page content streams, images, graphics state, Info metadata, and classic xref),
while direct ownership makes object ordering, byte limits, partial-write
behavior, and reproducibility straightforward to audit. PDF 1.7 was selected
because it is broadly supported and provides the standard transparency and
blend-mode facilities required by the current paint vocabulary. Streams use
deterministic zlib/Flate encoding, object numbers follow document declaration
order, and no timestamps, random identifiers, or current-time metadata are
written by default.

One point is one logical display-list unit. Page content starts with a
top-left-to-PDF coordinate conversion, then preserves display-list affine
transforms. `PageSpec` can additionally impose a validated content clipping
rectangle. The current mapping policy is:

| Display-list semantic | PDF policy |
| --- | --- |
| Save/restore, affine set/concat transform | Native graphics state and matrix |
| Rectangle/path fill, even-odd/non-zero rule | Native; quadratic curves become exact cubic curves |
| Center stroke, cap/join/miter, dash | Native |
| Intersect rectangle/path clip | Native |
| Solid opaque sRGBA SourceOver paint | Native color |
| Alpha, transparent image, or standard PDF blend mode | `NativePdf`: native color plus deduplicated ExtGState; `LinearMatch`: CPU page fallback or explicit error |
| sRGB RGBA8 image, opacity, reuse | Native Image XObject; alpha uses SMask; sampling choice is retained as the interpolation policy |
| Gradient/runtime shader, color filter, SaveLayer/image filter, difference clip, non-center stroke, path effect, non-PDF Porter-Duff mode, rational conic | Configurable whole-page CPU fallback, otherwise `Unsupported` |
| ICC-tagged image | `UnsupportedColorProfile`; profiles are never silently treated as sRGB |
| Glyph-run command | `UnsupportedText` in the first API because a DisplayList does not own its outline resolver |

Whole-page fallback has explicit DPI, pixel, and RGBA working-memory ceilings.
It renders into transparent straight-alpha RGBA8 through `skia-cpu`, embeds the
result as one image with an SMask when required, and does not depend on a GPU or
platform renderer. Fallback remains whole-page in this release; region/layer
rasterization is a future optimization. Text is intentionally not sent through
that fallback until the API can accept the matching `GlyphOutlineProvider`.
Consequently glyph content cannot disappear, but a glyph page currently fails
explicitly. A later outline strategy may preserve appearance and font
independence but will also document that outline text is not searchable;
searchable PDF text requires complete font subsetting, metrics, CID mapping,
and ToUnicode support and will not be added piecemeal.

PDF/A, tagged/structured PDF, outlines/bookmarks, encryption, and signatures
are not current features. XPS may later share a small multi-page lifecycle
crate with PDF once a second implementation proves that abstraction useful.
SVG remains a separate single-canvas output responsibility rather than being
forced into this multi-page API.

## Text implementation boundary

`skia-rs/text` owns portable font identities, ordered in-memory font collections,
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
Callers can use the cached `BuiltinTextBreakProvider`, backed by embedded
Knuth-Liang dictionaries, or plug custom language dictionaries into
`TextBreakProvider`; the layout engine validates UTF-8 grapheme boundaries and
supports either glyph-free soft breaks or synthetic visible hyphens without consuming source bytes. Layout options
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

System-font discovery, generic-family mapping, and language-preferred family
selection are available through the separate `skia-system-fonts` adapter;
variable-font instance policy and broader paragraph formatting remain upper
text-layout responsibilities. Portable `layout_outline_batches` and
`layout_decoration_batches` conversion lives in `skia-core`, producing ordinary
target-space paths and rectangles for any renderer. Its ordered
`text_layout_events` traversal is shared by CPU and DisplayList, while the GPU
atlas adapter uses the glyph-only traversal so decoration work cannot affect
atlas construction. GPU atlas text adaptation is available through the separate
`skia-gpu-text` adapter. For bitmap text,
`TextAtlasBuilder` rasterizes and packs a `TextLayout`, and `TextAtlas` converts
layout positions into owned generic quads without borrowing an encoder. The
caller then explicitly registers `into_gpu_atlas()` and records the quads with
`skia-gpu`. This keeps portable text geometry and GPU resource adaptation
separate from command ordering and hardware backends. The Metal
backend draws transformed/scissored solid rectangles, path-fill masks, Alpha8 masks, and color
glyphs through real shader pipelines; rectangle and glyph draws can sample
parent-linked R8 complex-clip masks rendered on the GPU. Destination snapshots
and programmable compositing cover every backend-neutral blend mode.
Local-space linear/radial gradients and pre-composite color filters use the
same paint uniforms across solid, path, stroke, and mask-glyph draws. Real
RGBA8 layer textures retain isolated command ranges; restore can run a color
filter or two-pass separable box blur before applying saved bounds, opacity,
blend mode, and complex clip. `StrokePath` shares deterministic
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
CPU Canvas, Metal, and Vulkan consume the same expanded triangle mesh; DisplayList and
generic GPU commands preserve the options, and software replay introduces no
backend-specific stroke policy.
Backend-neutral `SamplingOptions` similarly preserves nearest or bilinear
clamp-to-edge image reconstruction through DisplayList and GPU commands. CPU
uses checked affine inverse mapping and deterministic integer bilinear
interpolation; Metal applies the same texel-center convention to arbitrary
affine image draws.
Backend-neutral paint also carries bounded local-space linear/radial gradients,
Q16.16 color matrices and color filters. `SaveLayerOptions` records isolated
restore bounds, opacity, blend mode, and an optional color or box-blur image
filter. CPU Canvas and software GPU replay execute these semantics directly;
DisplayList and generic GPU commands retain the same layer boundaries and
image-paint state for hardware backends.
Backend-neutral `ClipOp` defines intersection and difference. CPU Canvas,
DisplayList replay, and the generic GPU encoder apply it to rectangles or paths.
Axis-aligned rectangle intersections retain a scissor fast path; CPU complex
clips use deterministic masks, while generic GPU commands retain immutable
parent-linked `GpuClipId` nodes. The software reference backend replays those
nodes through CPU masks, and Metal materializes only used nodes as transient R8
textures shared by subsequent rectangle, path-fill, and glyph draws in the submission.
CPU fill/stroke/clip plus Metal and Vulkan clip-edge generation consume the same bounded,
deterministic fixed-step curve flattener from `skia-tessellation`. Stroke
normalization, dashing, and cap/join/miter coverage also live there; CPU keeps
only device-pixel bounds and raster iteration, while backend-specific mask and
edge formats remain local.
`path_boolean` exposes bounded union, intersection, difference, and XOR over
flattened Q16.16 contours, including holes and self-intersections; empty set
results are represented as `None`, while non-empty output uses non-zero fill.
`skia-effects` provides `trim_path`, `corner_path`, `discrete_path`, and
`dash_path` as bounded path effects for normalized arc-length trimming,
deterministic quadratic corner rounding, seeded fixed-point contour
perturbation, and dashed centerlines. Trim
supports wrap-around intervals; corner radii are clamped to half of each
adjacent edge; discrete resampling keeps open endpoints and closed seams stable.
All implement the extensible `skia-core::PathEffect` contract and can run
left-to-right through `compose_path_effects` or nest through
`ComposePathEffect`; parallel results can be concatenated with
`SumPathEffect`. Input transforms are never reapplied between stages. Core owns
the contract and resource ceilings, while the concrete transformation
algorithms live in `skia-effects` and reuse `skia-tessellation`.
`PathEffectHandle` gives a `Paint` shared, cloneable ownership of one such
effect together with its resource limits. A paint holding a path effect is no
longer `Copy`; it remains `Clone`, `Eq`, and `Hash`, with handle equality based
on shared implementation identity and limits. CPU Canvas and DisplayList
replay expand the effect before stroking; the generic GPU encoder expands it at
recording time so hardware backends continue to receive an ordinary path.
`ShaderHandle` gives `Paint` shared ownership of a backend-neutral source
shader; gradients lower through it today. `ColorFilterHandle` gives `Paint` the
same model for existing value-backed color filters, and `ImageFilterHandle` lets
`SaveLayerOptions` retain a shared layer filter inside a DisplayList. They
deliberately lower back to established value effects at execution, so CPU,
Metal, and Vulkan keep their current implementations while the ownership
boundary is ready for future dynamic effect types.
The first runtime-shader tier adds a bounded typed IR with color uniforms,
local X/Y parameters, add/multiply/mix/clamp, and no loops, source text, host
callbacks, or texture access. `skia-core` validates the IR before a handle can
retain it; CPU Canvas and the software GPU evaluate it deterministically. Metal
and Vulkan encode the validated program and its uniforms into a fixed-size
packet and interpret it in their existing precompiled paint shaders, so source
draws never accept runtime MSL, SPIR-V, or other caller-provided shader source.
The portable limits (64 instructions, 16 color uniforms, and 16 registers)
keep that packet bounded. Each hardware backend retains a bounded
program-hash cache of encoded instruction streams and rebinds only uniforms per
draw. It also creates one native variant per validated program: Metal supplies
the packet through function constants, while Vulkan compiles a sealed internal
template to SPIR-V on its first cache miss. Neither path accepts executable
source text from callers; the ordinary precompiled VM remains the generic
fallback.
`skia-tessellation::stroke_to_path` produces a
deterministic non-zero triangle-fill path.

## Path implementation layout

The public `Path` and `PathBuilder` API is implemented in `skia-rs/path/src/path.rs` and
re-exported by the crate's thin `lib.rs` entry point.
Algorithm families are split beneath it so construction contracts do not become
coupled to geometry queries or contour processing:

- `path/arc.rs` owns public ellipse-arc construction and continuation methods.
- `path/bounds.rs` owns conservative and polynomial-Bézier extrema bounds helpers.
- `path/reverse.rs` owns contour parsing and reverse traversal.
- `path/math.rs` owns checked fixed-point scalar operations shared by path code.

Backend consumers must not add their own Bézier flattening. The reusable
`PathFlattener`, output ceilings, and flattened contour representation live in
`skia-rs/tessellation/src/flatten.rs`, ready for Metal, Vulkan, WebGPU, and CPU
consumers without exposing backend command or buffer types.

## GPU implementation layout

`skia-rs/gpu` is the public renderer-integration SPI, not a high-level drawing
API. Its thin `src/lib.rs` router re-exports contracts grouped by responsibility:

- `backend.rs` owns the backend trait and operational error boundary.
- `surface.rs` owns portable target descriptors and formats.
- `limits.rs` owns command ceilings and device-reported capabilities.
- `resource.rs` owns command-local IDs, atlas resources, glyph quads, and clip nodes.
- `command.rs` owns immutable commands and command buffers.
- `encoder.rs` owns stateful, bounded command recording.
- `software.rs` is the feature-gated conformance oracle, not a hardware backend.

Native handles, descriptor layouts, shaders, queues, and backend caches remain
inside `gpu/metal` or `gpu/vulkan`; they are not part of this SPI.
