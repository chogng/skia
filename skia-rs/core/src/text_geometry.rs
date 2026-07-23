use crate::{
    GlyphOutlineProvider, GlyphRun, Path, Point, Rect, Scalar, SkiaError, SkiaErrorCode,
    TextDecorationMetrics, TextDecorationStyle, TextError, TextErrorCode, TextLayout, TextStyleId,
    Transform, glyph_outline_path, text_decoration_rects,
};

/// One renderer-neutral item in a laid-out text block's visual draw order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextLayoutEvent<'a> {
    /// One positioned shaped run, before its line's decorations.
    GlyphRun {
        /// Caller-defined paint/style identity.
        style_id: TextStyleId,
        /// Immutable shaped glyph data.
        run: &'a GlyphRun,
        /// Target-space origin for the run.
        origin: Point,
        /// Per-glyph justification or spacing offsets.
        offsets_x_bits: &'a [i32],
    },
    /// One resolved target-space underline or strike-through rectangle.
    Decoration {
        /// Caller-defined paint/style identity.
        style_id: TextStyleId,
        /// Portable target-space decoration geometry.
        rect: Rect,
    },
}

/// Expands a layout into renderer-neutral events in visual drawing order.
///
/// Every line emits its shaped runs first and its decoration rectangles second.
/// Consumers can therefore share layout traversal without changing compositing
/// order. Events borrow glyph data from `layout` and own only small geometry.
pub fn text_layout_events(
    layout: &TextLayout,
    origin: Point,
) -> Result<Vec<TextLayoutEvent<'_>>, SkiaError> {
    collect_text_layout_events(layout, origin, true, true)
}

/// Expands only positioned shaped runs from a layout.
///
/// This avoids decoration work for atlas builders and other glyph-only
/// consumers while retaining the same checked run origins and spacing offsets.
pub fn text_layout_glyph_events(
    layout: &TextLayout,
    origin: Point,
) -> Result<Vec<TextLayoutEvent<'_>>, SkiaError> {
    collect_text_layout_events(layout, origin, true, false)
}

fn collect_text_layout_events(
    layout: &TextLayout,
    origin: Point,
    include_glyphs: bool,
    include_decorations: bool,
) -> Result<Vec<TextLayoutEvent<'_>>, SkiaError> {
    let mut events = Vec::new();
    for line in layout.lines() {
        let line_x = origin
            .x()
            .bits()
            .checked_add(line.offset_x_bits())
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
        let baseline_y = origin
            .y()
            .bits()
            .checked_add(line.baseline_y_bits())
            .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
        if include_glyphs && let Some(paragraph) = line.paragraph() {
            for shaped in paragraph.runs() {
                let run = shaped.glyph_run();
                if shaped.glyph_offsets_x_bits().len() != run.glyphs().len() {
                    return Err(SkiaError::new(SkiaErrorCode::InvalidResource));
                }
                let run_x = line_x
                    .checked_add(shaped.origin_x_bits())
                    .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
                push_event(
                    &mut events,
                    TextLayoutEvent::GlyphRun {
                        style_id: shaped.style_id(),
                        run,
                        origin: Point::new(Scalar::from_bits(run_x), Scalar::from_bits(baseline_y)),
                        offsets_x_bits: shaped.glyph_offsets_x_bits(),
                    },
                )?;
            }
        }
        if !include_decorations {
            continue;
        }
        if line.advance_x_bits() <= 0 {
            continue;
        }
        if line.decoration_segments().is_empty() {
            let right = line_x
                .checked_add(line.advance_x_bits())
                .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
            for metrics in [line.underline_metrics(), line.strike_through_metrics()]
                .into_iter()
                .flatten()
            {
                push_decoration_events(
                    &mut events,
                    TextStyleId::DEFAULT,
                    line_x,
                    right,
                    baseline_y,
                    metrics,
                    line.decoration_style(),
                )?;
            }
            continue;
        }
        for segment in line.decoration_segments() {
            let left = line_x
                .checked_add(segment.left_bits())
                .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
            let right = line_x
                .checked_add(segment.right_bits())
                .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
            for metrics in [
                segment.underline_metrics(),
                segment.strike_through_metrics(),
            ]
            .into_iter()
            .flatten()
            {
                push_decoration_events(
                    &mut events,
                    segment.style_id(),
                    left,
                    right,
                    baseline_y,
                    metrics,
                    segment.decoration_style(),
                )?;
            }
        }
    }
    Ok(events)
}

/// One contiguous batch of target-space glyph outline paths sharing a style.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextOutlineBatch {
    style_id: TextStyleId,
    paths: Vec<Path>,
}

impl TextOutlineBatch {
    /// Returns the caller-defined paint/style identity for this batch.
    pub const fn style_id(&self) -> TextStyleId {
        self.style_id
    }

    /// Borrows target-space glyph paths in visual drawing order.
    pub fn paths(&self) -> &[Path] {
        &self.paths
    }

    /// Moves target-space glyph paths out of this batch.
    pub fn into_paths(self) -> Vec<Path> {
        self.paths
    }
}

/// Converts a laid-out text block into per-style target-space vector paths.
///
/// Empty or missing outlines are skipped. Layout line offsets, run origins,
/// justification offsets, glyph positions, and `origin` are baked into each
/// path. The result is portable drawing geometry with no renderer dependency.
pub fn layout_outline_batches(
    layout: &TextLayout,
    provider: &impl GlyphOutlineProvider,
    origin: Point,
) -> Result<Vec<TextOutlineBatch>, SkiaError> {
    let mut batches: Vec<TextOutlineBatch> = Vec::new();
    for event in collect_text_layout_events(layout, origin, true, false)? {
        let TextLayoutEvent::GlyphRun {
            style_id,
            run,
            origin,
            offsets_x_bits,
        } = event
        else {
            continue;
        };
        let mut paths = Vec::new();
        for (glyph, offset_x) in run.glyphs().iter().zip(offsets_x_bits) {
            let Some(outline) = provider
                .glyph_outline(run.font(), glyph.glyph())
                .map_err(map_text_error)?
            else {
                continue;
            };
            if outline.font() != run.font() || outline.glyph() != glyph.glyph() {
                return Err(SkiaError::new(SkiaErrorCode::InvalidResource));
            }
            let Some(path) = glyph_outline_path(run, *glyph, &outline)? else {
                continue;
            };
            let x = origin
                .x()
                .bits()
                .checked_add(*offset_x)
                .ok_or(SkiaError::new(SkiaErrorCode::NumericOverflow))?;
            let path = path.transformed(Transform::translate(Scalar::from_bits(x), origin.y()))?;
            paths
                .try_reserve(1)
                .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
            paths.push(path);
        }
        if paths.is_empty() {
            continue;
        }
        if let Some(batch) = batches.last_mut()
            && batch.style_id == style_id
        {
            batch
                .paths
                .try_reserve(paths.len())
                .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
            batch.paths.extend(paths);
        } else {
            batches
                .try_reserve(1)
                .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
            batches.push(TextOutlineBatch { style_id, paths });
        }
    }
    Ok(batches)
}

/// One contiguous batch of target-space text-decoration rectangles sharing a style.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextDecorationBatch {
    style_id: TextStyleId,
    rects: Vec<Rect>,
}

impl TextDecorationBatch {
    /// Returns the caller-defined paint/style identity for this batch.
    pub const fn style_id(&self) -> TextStyleId {
        self.style_id
    }

    /// Borrows decoration rectangles in visual drawing order.
    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }

    /// Moves the decoration rectangles out of this batch.
    pub fn into_rects(self) -> Vec<Rect> {
        self.rects
    }
}

/// Converts resolved underline and strike-through metrics into target-space rectangles.
///
/// Adjacent output using the same [`TextStyleId`] is coalesced into one batch.
/// The result is portable drawing geometry with no renderer dependency.
pub fn layout_decoration_batches(
    layout: &TextLayout,
    origin: Point,
) -> Result<Vec<TextDecorationBatch>, SkiaError> {
    let mut batches = Vec::new();
    for event in collect_text_layout_events(layout, origin, false, true)? {
        if let TextLayoutEvent::Decoration { style_id, rect } = event {
            push_decoration(&mut batches, style_id, rect)?;
        }
    }
    Ok(batches)
}

fn push_decoration_events<'a>(
    events: &mut Vec<TextLayoutEvent<'a>>,
    style_id: TextStyleId,
    left_bits: i32,
    right_bits: i32,
    baseline_bits: i32,
    metrics: TextDecorationMetrics,
    style: TextDecorationStyle,
) -> Result<(), SkiaError> {
    for rect in text_decoration_rects(left_bits, right_bits, baseline_bits, metrics, style)
        .map_err(map_text_error)?
    {
        let rect = Rect::new(
            Scalar::from_bits(rect.left_bits()),
            Scalar::from_bits(rect.top_bits()),
            Scalar::from_bits(rect.right_bits()),
            Scalar::from_bits(rect.bottom_bits()),
        )?;
        push_event(events, TextLayoutEvent::Decoration { style_id, rect })?;
    }
    Ok(())
}

fn push_event<'a>(
    events: &mut Vec<TextLayoutEvent<'a>>,
    event: TextLayoutEvent<'a>,
) -> Result<(), SkiaError> {
    events
        .try_reserve(1)
        .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
    events.push(event);
    Ok(())
}

fn push_decoration(
    batches: &mut Vec<TextDecorationBatch>,
    style_id: TextStyleId,
    rect: Rect,
) -> Result<(), SkiaError> {
    if let Some(batch) = batches.last_mut()
        && batch.style_id == style_id
    {
        batch
            .rects
            .try_reserve(1)
            .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
        batch.rects.push(rect);
        return Ok(());
    }

    batches
        .try_reserve(1)
        .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
    let mut rects = Vec::new();
    rects
        .try_reserve(1)
        .map_err(|_| SkiaError::new(SkiaErrorCode::AllocationFailed))?;
    rects.push(rect);
    batches.push(TextDecorationBatch { style_id, rects });
    Ok(())
}

fn map_text_error(error: TextError) -> SkiaError {
    let code = match error.code() {
        TextErrorCode::AllocationFailed => SkiaErrorCode::AllocationFailed,
        TextErrorCode::NumericOverflow => SkiaErrorCode::NumericOverflow,
        TextErrorCode::ResourceLimit => SkiaErrorCode::ResourceLimit,
        _ => SkiaErrorCode::TextResolverFailed,
    };
    SkiaError::new(code)
}
