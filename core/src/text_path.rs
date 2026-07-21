use crate::{
    GlyphOutline, GlyphRun, OutlinePoint, OutlineSegment, Path, PathBuilder, Point,
    PositionedGlyph, Scalar, SkiaError, SkiaErrorCode, TextUnit,
};

/// Converts one positioned glyph outline into an ordinary canvas-space path.
///
/// Design coordinates and the glyph position are scaled by the run's font size
/// and units-per-em. Empty outlines return `None`; all other geometry uses the
/// same checked Q16.16 path contract as caller-authored vector shapes.
pub fn glyph_outline_path(
    run: &GlyphRun,
    glyph: PositionedGlyph,
    outline: &GlyphOutline,
) -> Result<Option<Path>, SkiaError> {
    if outline.segments().is_empty() {
        return Ok(None);
    }
    let mut builder = PathBuilder::new(outline.segments().len())?;
    for segment in outline.segments() {
        match *segment {
            OutlineSegment::MoveTo(point) => {
                builder.move_to(scaled_outline_point(run, glyph, point)?)?
            }
            OutlineSegment::LineTo(point) => {
                builder.line_to(scaled_outline_point(run, glyph, point)?)?
            }
            OutlineSegment::QuadTo { control, end } => builder.quad_to(
                scaled_outline_point(run, glyph, control)?,
                scaled_outline_point(run, glyph, end)?,
            )?,
            OutlineSegment::CubicTo {
                first_control,
                second_control,
                end,
            } => builder.cubic_to(
                scaled_outline_point(run, glyph, first_control)?,
                scaled_outline_point(run, glyph, second_control)?,
                scaled_outline_point(run, glyph, end)?,
            )?,
            OutlineSegment::Close => builder.close()?,
        }
    }
    builder.finish().map(Some)
}

fn scaled_outline_point(
    run: &GlyphRun,
    glyph: PositionedGlyph,
    point: OutlinePoint,
) -> Result<Point, SkiaError> {
    Ok(Point::new(
        scaled_text_coordinate(point.x(), glyph.x(), run)?,
        scaled_text_coordinate(point.y(), glyph.y(), run)?,
    ))
}

fn scaled_text_coordinate(
    outline: TextUnit,
    position: TextUnit,
    run: &GlyphRun,
) -> Result<Scalar, SkiaError> {
    let design = i64::from(outline.bits())
        .checked_add(i64::from(position.bits()))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let numerator = i128::from(design)
        .checked_mul(i128::from(run.font_size_bits()))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let denominator = i128::from(64_i32)
        .checked_mul(i128::from(run.units_per_em()))
        .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
    let rounded = if numerator >= 0 {
        numerator
            .checked_add(denominator / 2)
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?
            / denominator
    } else {
        -((-numerator
            .checked_add(denominator / 2)
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?)
            / denominator)
    };
    i32::try_from(rounded)
        .map(Scalar::from_bits)
        .map_err(|_| SkiaError::new(SkiaErrorCode::NumericOverflow))
}
