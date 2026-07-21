# Vulkan backend bring-up

`skia-vulkan` is a real offscreen Vulkan backend. The initial bring-up loads
the system Vulkan loader dynamically, selects a graphics queue, allocates an
optimal-tiled RGBA8 image, executes `GpuCommand::Clear`, copies through a
host-visible staging buffer, and verifies exact RGBA8 readback. Unsupported
draw commands return `UnsupportedCommand`; there is no CPU fallback.

## Windows verification

Follow [`WINDOWS_VALIDATION.md`](WINDOWS_VALIDATION.md) for the complete
copy-paste runbook. It covers prerequisites, the basic forced-device run, the
Khronos Validation Layer run, expected output, failure diagnosis, and the exact
information to report.

The shortest basic check from a Developer PowerShell in the repository root is:

```powershell
$env:SKIA_REQUIRE_VULKAN_DEVICE = "1"
cargo test -p skia-vulkan -- --nocapture --test-threads=1
```

After installing the Vulkan SDK, require `VK_LAYER_KHRONOS_validation` with:

```powershell
$env:SKIA_REQUIRE_VULKAN_DEVICE = "1"
$env:SKIA_VULKAN_VALIDATION = "1"
cargo test -p skia-vulkan -- --nocapture --test-threads=1
```

Both runs must report two passing tests and print the selected device name.
Forced runs fail rather than silently skipping unavailable loader, device, or
validation-layer prerequisites.

This phase intentionally does not create a window or swapchain. Presentation,
shader pipelines, descriptor layouts, generic mesh drawing, and GPU complex
clip masks are subsequent stages built on this device/surface foundation.
