/// Backend category selected by a GPU integration test.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GpuContextType {
    /// Deterministic CPU replay used as the pixel oracle.
    Software,
    /// Native Vulkan device and queue.
    Vulkan,
    /// Native Metal device and command queue.
    Metal,
}

impl GpuContextType {
    /// Returns a stable display name suitable for test diagnostics.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Software => "software",
            Self::Vulkan => "vulkan",
            Self::Metal => "metal",
        }
    }

    /// Returns whether this context type owns a native GPU device.
    pub const fn is_native(self) -> bool {
        !matches!(self, Self::Software)
    }

    /// Returns whether this context can issue rendering commands.
    pub const fn is_rendering(self) -> bool {
        true
    }
}
