/// Boolean operation applied when extending the current clip.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ClipOp {
    /// Keeps pixels that are inside both the current clip and the new geometry.
    Intersect,
    /// Removes pixels inside the new geometry from the current clip.
    Difference,
}
