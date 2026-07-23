use std::fmt;

use skia_core::{SkiaError, SkiaErrorCode, TextError, TextErrorCode};
use skia_gpu::{GpuCommandError, GpuCommandErrorCode};

/// Stable machine-readable text-to-GPU adapter failure.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TextGpuErrorCode {
    /// Atlas dimensions or glyph ceilings are invalid.
    InvalidLimits,
    /// A coordinate or byte-size calculation overflowed.
    NumericOverflow,
    /// Atlas storage or glyph capacity was exhausted.
    ResourceLimit,
    /// A font, layout, or atlas resource was inconsistent.
    InvalidResource,
    /// A bounded allocation could not be reserved.
    AllocationFailed,
}

/// Source-redacted text-to-GPU adapter error.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextGpuError {
    code: TextGpuErrorCode,
}

impl TextGpuError {
    pub(crate) const fn new(code: TextGpuErrorCode) -> Self {
        Self { code }
    }

    /// Returns the stable failure code.
    pub const fn code(self) -> TextGpuErrorCode {
        self.code
    }
}

impl fmt::Display for TextGpuError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.code)
    }
}

impl std::error::Error for TextGpuError {}

impl From<GpuCommandError> for TextGpuError {
    fn from(error: GpuCommandError) -> Self {
        let code = match error.code() {
            GpuCommandErrorCode::InvalidLimits => TextGpuErrorCode::InvalidLimits,
            GpuCommandErrorCode::NumericOverflow => TextGpuErrorCode::NumericOverflow,
            GpuCommandErrorCode::ResourceLimit => TextGpuErrorCode::ResourceLimit,
            GpuCommandErrorCode::AllocationFailed => TextGpuErrorCode::AllocationFailed,
            GpuCommandErrorCode::InvalidSurface
            | GpuCommandErrorCode::RestoreUnderflow
            | GpuCommandErrorCode::UnsupportedTransform
            | GpuCommandErrorCode::InvalidResource => TextGpuErrorCode::InvalidResource,
        };
        Self::new(code)
    }
}

pub(crate) fn map_text_error(error: TextError) -> TextGpuError {
    let code = match error.code() {
        TextErrorCode::AllocationFailed => TextGpuErrorCode::AllocationFailed,
        TextErrorCode::NumericOverflow => TextGpuErrorCode::NumericOverflow,
        TextErrorCode::ResourceLimit => TextGpuErrorCode::ResourceLimit,
        _ => TextGpuErrorCode::InvalidResource,
    };
    TextGpuError::new(code)
}

pub(crate) fn map_skia_error(error: SkiaError) -> TextGpuError {
    let code = match error.code() {
        SkiaErrorCode::NumericOverflow => TextGpuErrorCode::NumericOverflow,
        SkiaErrorCode::ResourceLimit => TextGpuErrorCode::ResourceLimit,
        SkiaErrorCode::AllocationFailed => TextGpuErrorCode::AllocationFailed,
        _ => TextGpuErrorCode::InvalidResource,
    };
    TextGpuError::new(code)
}
