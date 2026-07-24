# GPU test resource utilities

This package is the shared resource layer for GPU integration tests. Its role
matches upstream `tools/gpu`: tests construct portable images and surfaces,
retain resources for the full submission lifetime, and track completion without
duplicating that bookkeeping in every backend test.

The current Rust GPU API exposes synchronous submission and normalized RGBA8
images. Accordingly, this first implementation provides:

- backend/context classification;
- backend surface construction;
- managed RGBA8 image fixtures, including solid and checkerboard sources;
- narrow two-color BC1 fixture compression;
- submission completion tracking.

Native `VkImage`/Metal texture allocation remains inside the backend crates,
where ownership and command-buffer lifetime are enforced. Additional compressed
formats and multi-plane YUV helpers belong here once `skia-image` exposes their
resource types; they are deliberately not represented as lossy RGBA stand-ins.
