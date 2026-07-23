use std::fmt;

/// Stable machine-readable Vulkan backend failure.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum VulkanErrorCode {
    /// No Vulkan loader could be opened on this machine.
    LoaderUnavailable,
    /// Vulkan instance creation failed.
    InstanceCreationFailed,
    /// Validation was required but `VK_LAYER_KHRONOS_validation` is unavailable.
    ValidationUnavailable,
    /// No physical device with a compute-capable queue was available.
    DeviceUnavailable,
    /// Logical-device, queue, or command-pool creation failed.
    DeviceCreationFailed,
    /// Offscreen target or device-memory allocation failed.
    SurfaceAllocationFailed,
    /// The generated SPIR-V shader module could not be loaded.
    ShaderModuleFailed,
    /// The Vulkan compute pipeline or its descriptor layout could not be created.
    PipelineCreationFailed,
    /// The command buffer contains an invalid or unsupported command.
    UnsupportedCommand,
    /// Host-visible staging allocation, mapping, or device upload failed.
    UploadFailed,
    /// Command recording, queue submission, or synchronization failed.
    SubmissionFailed,
    /// Staging allocation, buffer copy, mapping, or readback failed.
    ReadbackFailed,
}

/// Source-redacted Vulkan backend error.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct VulkanError {
    code: VulkanErrorCode,
}

impl VulkanError {
    pub(crate) const fn new(code: VulkanErrorCode) -> Self {
        Self { code }
    }

    /// Returns the stable failure code.
    pub const fn code(self) -> VulkanErrorCode {
        self.code
    }
}

impl fmt::Display for VulkanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.code)
    }
}

impl std::error::Error for VulkanError {}
