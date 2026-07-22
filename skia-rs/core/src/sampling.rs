/// Image reconstruction filter selected for bitmap draws.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SamplingFilter {
    /// Selects the nearest texel at each destination pixel center.
    Nearest,
    /// Bilinearly interpolates the four texels around each source position.
    Linear,
}

/// Backend-neutral image sampling configuration.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SamplingOptions {
    filter: SamplingFilter,
}

impl SamplingOptions {
    /// Nearest-neighbor sampling with clamp-to-edge addressing.
    pub const NEAREST: Self = Self::new(SamplingFilter::Nearest);
    /// Bilinear sampling with clamp-to-edge addressing.
    pub const LINEAR: Self = Self::new(SamplingFilter::Linear);

    /// Creates sampling options for one reconstruction filter.
    pub const fn new(filter: SamplingFilter) -> Self {
        Self { filter }
    }

    /// Returns the selected reconstruction filter.
    pub const fn filter(self) -> SamplingFilter {
        self.filter
    }
}

impl Default for SamplingOptions {
    fn default() -> Self {
        Self::NEAREST
    }
}
