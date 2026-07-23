# Vulkan backend

`skia-vulkan` is the cross-platform Vulkan execution adapter for `skia-gpu`.
It dynamically loads the system Vulkan loader, selects a compute-capable
device and queue, and renders into a device-owned RGBA8 storage target.

## Current implementation

- Target-wide clears use a native Vulkan transfer command.
- A build-time WGSL-to-SPIR-V step produces the Vulkan compute shader.
- Rectangles, paths, strokes, images, glyphs, complex clips, isolated layers,
  gradients, filters, blur, sampling, and every portable blend mode execute in
  the compute pipeline. The host only interprets commands, expands geometry,
  and uploads immutable resources.
- Surface contents are preserved across submissions.
- Exact RGBA8 readback uses a separate Vulkan staging buffer.
- `skia-cpu` and the software GPU backend are test-only pixel oracles; neither
  is present in the production dependency graph.

The backend is currently offscreen only. It does not create a window or
swapchain; presentation remains a separate platform-integration concern.

## Testing

Run the test suite from `skia-rs/`:

```console
cargo test -p skia-vulkan
```

Hardware tests skip by default when the Vulkan loader or a graphics-capable
device is unavailable.

### Windows hardware validation

Install a current vendor GPU driver, then require a real Vulkan device from a
Developer PowerShell:

```powershell
Remove-Item Env:SKIA_VULKAN_VALIDATION -ErrorAction SilentlyContinue
$env:SKIA_REQUIRE_VULKAN_DEVICE = "1"
cargo test -p skia-vulkan -- --nocapture --test-threads=1
```

To also enable the Khronos Validation Layer, install the Vulkan SDK and rerun:

```powershell
$env:SKIA_VULKAN_VALIDATION = "1"
cargo test -p skia-vulkan -- --nocapture --test-threads=1
```

Both runs must pass all tests and print `Vulkan device: <device name>`. The
validation run fails with `ValidationUnavailable` if
`VK_LAYER_KHRONOS_validation` is not installed. The backend enables the layer
but does not yet register a `VK_EXT_debug_utils` callback.

The switches are enabled by their presence, not their value. Remove them after
testing instead of setting them to `0`:

```powershell
Remove-Item Env:SKIA_REQUIRE_VULKAN_DEVICE -ErrorAction SilentlyContinue
Remove-Item Env:SKIA_VULKAN_VALIDATION -ErrorAction SilentlyContinue
```
