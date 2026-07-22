# Vulkan backend

`skia-vulkan` is the cross-platform Vulkan execution adapter for `skia-gpu`.
It dynamically loads the system Vulkan loader, selects a graphics-capable
device and queue, and renders into a device-owned, optimal-tiled RGBA8 image.

## Current implementation

- Target-wide clears use a native Vulkan transfer command.
- The complete portable `GpuCommand` vocabulary is composed deterministically
  and uploaded through a host-visible staging buffer.
- Surface contents are preserved across submissions.
- Exact RGBA8 readback uses a separate Vulkan staging buffer.

The backend is currently offscreen only. It does not create a window or
swapchain. Presentation and fully native shader and descriptor pipelines are
separate work.

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
