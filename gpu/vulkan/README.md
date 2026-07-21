# Vulkan backend bring-up

`skia-vulkan` is a real offscreen Vulkan backend. The initial bring-up loads
the system Vulkan loader dynamically, selects a graphics queue, allocates an
optimal-tiled RGBA8 image, executes `GpuCommand::Clear`, copies through a
host-visible staging buffer, and verifies exact RGBA8 readback. Unsupported
draw commands return `UnsupportedCommand`; there is no CPU fallback.

## Windows verification

Install a current GPU driver with Vulkan runtime support. The Vulkan SDK is
optional for the basic test and required when enabling the Khronos validation
layer. From a Developer PowerShell in the repository root, run:

```powershell
$env:SKIA_REQUIRE_VULKAN_DEVICE = "1"
cargo test -p skia-vulkan -- --nocapture
```

The suite must report two passing tests and print the selected device name. To
require `VK_LAYER_KHRONOS_validation` during the same run:

```powershell
$env:SKIA_REQUIRE_VULKAN_DEVICE = "1"
$env:SKIA_VULKAN_VALIDATION = "1"
cargo test -p skia-vulkan -- --nocapture
```

If the loader/device or requested validation layer is unavailable, the forced
run fails instead of silently skipping. Remove the variables afterward with:

```powershell
Remove-Item Env:SKIA_REQUIRE_VULKAN_DEVICE
Remove-Item Env:SKIA_VULKAN_VALIDATION
```

This phase intentionally does not create a window or swapchain. Presentation,
shader pipelines, descriptor layouts, generic mesh drawing, and GPU complex
clip masks are subsequent stages built on this device/surface foundation.
