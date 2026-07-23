use crate::{Path, SkiaError, SkiaErrorCode, Transform};

const DEFAULT_CURVE_STEPS: u32 = 16;

/// Extensible backend-neutral operation that maps one path to another bounded path.
///
/// Concrete built-in effects live in `skia-effects`. Keeping this contract in
/// `skia-core` lets paints, display lists, and callers refer to path effects
/// without making the core crate depend on any concrete effect implementation.
pub trait PathEffect: Send + Sync {
    /// Applies this effect and returns `None` when it removes all geometry.
    fn apply(
        &self,
        path: &Path,
        transform: Transform,
        limits: PathEffectLimits,
    ) -> Result<Option<Path>, SkiaError>;
}

/// Resource ceilings for one path-effect operation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PathEffectLimits {
    max_contours: usize,
    max_points: usize,
    max_output_verbs: usize,
    curve_steps: u32,
}

impl PathEffectLimits {
    /// Creates positive flattening and output ceilings.
    pub fn new(
        max_contours: usize,
        max_points: usize,
        max_output_verbs: usize,
        curve_steps: u32,
    ) -> Result<Self, SkiaError> {
        if max_contours == 0 || max_points == 0 || max_output_verbs == 0 || curve_steps == 0 {
            return Err(SkiaError::new(SkiaErrorCode::InvalidLimits));
        }
        Ok(Self {
            max_contours,
            max_points,
            max_output_verbs,
            curve_steps,
        })
    }

    /// Returns the maximum number of flattened contours.
    pub const fn max_contours(self) -> usize {
        self.max_contours
    }

    /// Returns the maximum number of flattened points.
    pub const fn max_points(self) -> usize {
        self.max_points
    }

    /// Returns the maximum number of output path verbs.
    pub const fn max_output_verbs(self) -> usize {
        self.max_output_verbs
    }

    /// Returns the fixed number of line segments emitted for each curve verb.
    pub const fn curve_steps(self) -> u32 {
        self.curve_steps
    }
}

impl Default for PathEffectLimits {
    fn default() -> Self {
        Self {
            max_contours: 4_096,
            max_points: 1_048_576,
            max_output_verbs: 1_048_576,
            curve_steps: DEFAULT_CURVE_STEPS,
        }
    }
}

/// Applies one effect through the shared path-effect contract.
pub fn apply_path_effect(
    path: &Path,
    effect: &dyn PathEffect,
    transform: Transform,
    limits: PathEffectLimits,
) -> Result<Option<Path>, SkiaError> {
    let output = effect.apply(path, transform, limits)?;
    if output
        .as_ref()
        .is_some_and(|path| path.verbs().len() > limits.max_output_verbs)
    {
        return Err(SkiaError::new(SkiaErrorCode::ResourceLimit));
    }
    Ok(output)
}

/// Applies effects from left to right with the same per-stage resource limits.
///
/// The transform is consumed by the first stage only. An empty effect slice is
/// an identity pipeline: it returns a transformed path while still enforcing
/// `max_output_verbs`.
pub fn compose_path_effects(
    path: &Path,
    effects: &[&dyn PathEffect],
    transform: Transform,
    limits: PathEffectLimits,
) -> Result<Option<Path>, SkiaError> {
    let Some((first, rest)) = effects.split_first() else {
        if path.verbs().len() > limits.max_output_verbs {
            return Err(SkiaError::new(SkiaErrorCode::ResourceLimit));
        }
        return path.transformed(transform).map(Some);
    };
    let Some(mut current) = apply_path_effect(path, *first, transform, limits)? else {
        return Ok(None);
    };
    for effect in rest {
        let Some(next) = apply_path_effect(&current, *effect, Transform::IDENTITY, limits)? else {
            return Ok(None);
        };
        current = next;
    }
    Ok(Some(current))
}
