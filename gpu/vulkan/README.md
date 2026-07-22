# Vulkan backend bring-up

`skia-vulkan` is an offscreen Vulkan execution adapter. It loads the system
Vulkan loader dynamically, selects a graphics queue, allocates an optimal-tiled
RGBA8 image, executes target-wide clears natively, and supports the complete
portable `GpuCommand` vocabulary through deterministic composition followed by
a host-visible staging upload. The target remains device-owned, contents are
preserved across submissions, and exact RGBA8 readback uses a separate Vulkan
staging buffer.

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

Both runs must report all Vulkan tests passing and print the selected device name.
Forced runs fail rather than silently skipping unavailable loader, device, or
validation-layer prerequisites.

This phase intentionally does not create a window or swapchain. Presentation,
swapchain integration, and fully native shader/descriptor pipelines remain
separate work from this offscreen command adapter.
