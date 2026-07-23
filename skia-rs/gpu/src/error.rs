use std::fmt;

/// Stable machine-readable GPU command recording failure.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GpuCommandErrorCode {
    /// A command-buffer or backend ceiling is invalid.
    InvalidLimits,
    /// A GPU surface descriptor has an empty dimension.
    InvalidSurface,
    /// A state restore was requested without a matching save.
    RestoreUnderflow,
    /// The operation needs an unsupported transform mode.
    UnsupportedTransform,
    /// A transform or intermediate geometry calculation overflowed.
    NumericOverflow,
    /// Recording would exceed a configured ceiling.
    ResourceLimit,
    /// A command referred to a resource that is not registered in this encoder.
    InvalidResource,
    /// Recording could not reserve command storage.
    AllocationFailed,
}

/// Source-redacted GPU command recording or capability failure.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GpuCommandError {
    code: GpuCommandErrorCode,
}

impl GpuCommandError {
    pub(crate) const fn new(code: GpuCommandErrorCode) -> Self {
        Self { code }
    }

    /// Returns the stable failure code.
    pub const fn code(self) -> GpuCommandErrorCode {
        self.code
    }
}

impl fmt::Display for GpuCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.code)
    }
}

impl std::error::Error for GpuCommandError {}
