use std::{collections::HashMap, sync::Arc};

use skia_core::{
    BlendMode, ClipOp, Color, ColorFilter, FillRule, GradientGeometry, ImageFilter, Paint, Point,
    Rect, SamplingFilter, SaveLayerOptions, Scalar, TileMode, Transform,
};
use skia_gpu::{GpuClipGeometry, GpuClipId, GpuCommand, GpuCommandBuffer, GpuSurfaceDescriptor};
use skia_tessellation::{DEFAULT_CURVE_STEPS, FlatteningLimits, PathFlattener, stroke_mesh};

use crate::{
    VulkanError, VulkanErrorCode,
    renderer::VulkanRenderer,
    surface::{PixelBuffer, VulkanSurface},
};

const OP_SOLID: u32 = 1;
const OP_PATH: u32 = 2;
const OP_TRIANGLES: u32 = 3;
const OP_IMAGE: u32 = 4;
const OP_GLYPH: u32 = 5;
const OP_LAYER: u32 = 6;
const OP_CLIP: u32 = 7;
const OP_BLUR_X: u32 = 8;
const OP_BLUR_Y: u32 = 9;
const PARAMETER_WORDS: usize = 96;

pub(crate) fn submit(
    renderer: &VulkanRenderer,
    context: Arc<crate::context::VulkanContext>,
    surface: &mut VulkanSurface,
    commands: &GpuCommandBuffer,
) -> Result<(), VulkanError> {
    if commands
        .commands()
        .iter()
        .any(GpuCommand::requires_runtime_shader_lowering)
    {
        return Err(VulkanError::new(VulkanErrorCode::UnsupportedCommand));
    }
    let descriptor = surface.descriptor();
    let mut layers = Vec::<Layer>::new();
    let mut clips = HashMap::<GpuClipId, PixelBuffer>::new();
    for command in commands.commands() {
        match command {
            GpuCommand::Clear(color) => current_target(surface, &layers).clear(*color)?,
            GpuCommand::SaveLayer {
                options,
                transform,
                scissor,
                clip,
            } => {
                if let Some(id) = clip {
                    ensure_clip(
                        renderer,
                        context.clone(),
                        descriptor,
                        commands,
                        *id,
                        &mut clips,
                    )?;
                }
                let pixels = PixelBuffer::new(context.clone(), descriptor)?;
                pixels.clear(Color::TRANSPARENT)?;
                layers.push(Layer {
                    pixels,
                    options: options.clone(),
                    transform: *transform,
                    scissor: *scissor,
                    clip: *clip,
                });
            }
            GpuCommand::RestoreLayer => {
                let layer = layers
                    .pop()
                    .ok_or(VulkanError::new(VulkanErrorCode::UnsupportedCommand))?;
                let filtered = filter_layer(renderer, context.clone(), descriptor, &layer)?;
                let source = filtered.as_ref().unwrap_or(&layer.pixels);
                let target = current_target(surface, &layers);
                let clip = layer.clip.and_then(|id| clips.get(&id));
                let mut params = base_params(OP_LAYER, descriptor);
                params[4] = u32::from(layer.options.opacity());
                params[12] = blend_mode(layer.options.blend_mode());
                set_transform(&mut params, Transform::IDENTITY)?;
                set_rect(
                    &mut params,
                    layer_restore_bounds(layer.options.bounds(), layer.transform, descriptor)?,
                );
                set_scissor(&mut params, layer.scissor);
                params[7] = u32::from(clip.is_some());
                renderer.dispatch(target, Some(source), clip, &[], &params)?;
            }
            GpuCommand::FillRect {
                rect,
                paint,
                transform,
                scissor,
                clip,
            } => {
                let clip = resolve_clip(
                    renderer,
                    context.clone(),
                    descriptor,
                    commands,
                    *clip,
                    &mut clips,
                )?;
                let mut params = draw_params(OP_SOLID, descriptor, paint, *transform, *scissor)?;
                set_rect(&mut params, *rect);
                params[7] = u32::from(clip.is_some());
                renderer.dispatch(current_target(surface, &layers), None, clip, &[], &params)?;
            }
            GpuCommand::FillPath {
                path,
                rule,
                paint,
                transform,
                scissor,
                clip,
            } => {
                let path = commands
                    .path(*path)
                    .ok_or(VulkanError::new(VulkanErrorCode::UnsupportedCommand))?;
                let edges = path_edges(path, *transform)?;
                let clip = resolve_clip(
                    renderer,
                    context.clone(),
                    descriptor,
                    commands,
                    *clip,
                    &mut clips,
                )?;
                let mut params = draw_params(OP_PATH, descriptor, paint, *transform, *scissor)?;
                params[4] = u32::try_from(edges.len() / 4)
                    .map_err(|_| VulkanError::new(VulkanErrorCode::SubmissionFailed))?;
                params[5] = u32::from(matches!(rule, FillRule::EvenOdd));
                params[7] = u32::from(clip.is_some());
                renderer.dispatch(
                    current_target(surface, &layers),
                    None,
                    clip,
                    &edges,
                    &params,
                )?;
            }
            GpuCommand::StrokePath {
                path,
                options,
                paint,
                transform,
                scissor,
                clip,
            } => {
                let path = commands
                    .path(*path)
                    .ok_or(VulkanError::new(VulkanErrorCode::UnsupportedCommand))?;
                let limits = FlatteningLimits::for_path(path, DEFAULT_CURVE_STEPS)
                    .map_err(|_| VulkanError::new(VulkanErrorCode::SubmissionFailed))?;
                let contours = PathFlattener::new(limits)
                    .flatten(path, *transform)
                    .map_err(|_| VulkanError::new(VulkanErrorCode::SubmissionFailed))?;
                let mesh = stroke_mesh(contours.contours(), options)
                    .map_err(|_| VulkanError::new(VulkanErrorCode::SubmissionFailed))?;
                let mut triangles = Vec::with_capacity(mesh.vertices().len() * 2);
                for point in mesh.vertices() {
                    triangles.extend([scalar_word(point.x()), scalar_word(point.y())]);
                }
                let clip = resolve_clip(
                    renderer,
                    context.clone(),
                    descriptor,
                    commands,
                    *clip,
                    &mut clips,
                )?;
                let mut params =
                    draw_params(OP_TRIANGLES, descriptor, paint, *transform, *scissor)?;
                params[4] = u32::try_from(mesh.vertices().len() / 3)
                    .map_err(|_| VulkanError::new(VulkanErrorCode::SubmissionFailed))?;
                params[7] = u32::from(clip.is_some());
                renderer.dispatch(
                    current_target(surface, &layers),
                    None,
                    clip,
                    &triangles,
                    &params,
                )?;
            }
            GpuCommand::DrawImage {
                image,
                destination,
                opacity,
                sampling,
                paint,
                transform,
                scissor,
                clip,
                ..
            } => {
                let image = commands
                    .image(*image)
                    .ok_or(VulkanError::new(VulkanErrorCode::UnsupportedCommand))?;
                let pixels = rgba_words(image.pixels());
                let clip = resolve_clip(
                    renderer,
                    context.clone(),
                    descriptor,
                    commands,
                    *clip,
                    &mut clips,
                )?;
                let mut params = draw_params(OP_IMAGE, descriptor, paint, *transform, *scissor)?;
                params[32] = 0;
                set_rect(&mut params, *destination);
                params[4] = multiply_255(*opacity, paint.color().alpha());
                params[13] = u32::from(matches!(sampling.filter(), SamplingFilter::Linear));
                params[7] = u32::from(clip.is_some());
                set_image(
                    &mut params,
                    image.width(),
                    image.height(),
                    0,
                    0,
                    image.width(),
                    image.height(),
                );
                renderer.dispatch(
                    current_target(surface, &layers),
                    None,
                    clip,
                    &pixels,
                    &params,
                )?;
            }
            GpuCommand::DrawGlyphs {
                atlas,
                glyphs,
                paint,
                transform,
                scissor,
                clip,
            } => {
                let atlas = commands
                    .glyph_atlas(*atlas)
                    .ok_or(VulkanError::new(VulkanErrorCode::UnsupportedCommand))?;
                let image = atlas.image();
                let pixels = rgba_words(image.pixels());
                let clip = resolve_clip(
                    renderer,
                    context.clone(),
                    descriptor,
                    commands,
                    *clip,
                    &mut clips,
                )?;
                for glyph in glyphs {
                    let mut params =
                        draw_params(OP_GLYPH, descriptor, paint, *transform, *scissor)?;
                    set_rect(&mut params, glyph.destination());
                    params[4] = u32::from(paint.color().alpha());
                    params[5] = u32::from(glyph.is_mask());
                    params[7] = u32::from(clip.is_some());
                    let source = glyph.source();
                    set_image(
                        &mut params,
                        image.width(),
                        image.height(),
                        source.x(),
                        source.y(),
                        source.width(),
                        source.height(),
                    );
                    renderer.dispatch(
                        current_target(surface, &layers),
                        None,
                        clip,
                        &pixels,
                        &params,
                    )?;
                }
            }
        }
    }
    if !layers.is_empty() {
        return Err(VulkanError::new(VulkanErrorCode::UnsupportedCommand));
    }
    surface.mark_initialized();
    Ok(())
}

struct Layer {
    pixels: PixelBuffer,
    options: SaveLayerOptions,
    transform: Transform,
    scissor: Option<Rect>,
    clip: Option<GpuClipId>,
}

fn filter_layer(
    renderer: &VulkanRenderer,
    context: Arc<crate::context::VulkanContext>,
    descriptor: GpuSurfaceDescriptor,
    layer: &Layer,
) -> Result<Option<PixelBuffer>, VulkanError> {
    match layer.options.filter() {
        None => Ok(None),
        Some(ImageFilter::BoxBlur { radius }) => {
            let horizontal = PixelBuffer::new(context.clone(), descriptor)?;
            let vertical = PixelBuffer::new(context, descriptor)?;
            let mut params = base_params(OP_BLUR_X, descriptor);
            params[4] = u32::from(radius);
            renderer.dispatch(&horizontal, Some(&layer.pixels), None, &[], &params)?;
            params[0] = OP_BLUR_Y;
            renderer.dispatch(&vertical, Some(&horizontal), None, &[], &params)?;
            Ok(Some(vertical))
        }
        Some(ImageFilter::Color(filter)) => {
            let output = PixelBuffer::new(context, descriptor)?;
            output.clear(Color::TRANSPARENT)?;
            let mut params = base_params(OP_LAYER, descriptor);
            params[4] = u32::from(u8::MAX);
            params[12] = blend_mode(BlendMode::Source);
            set_transform(&mut params, Transform::IDENTITY)?;
            set_rect(&mut params, full_rect(descriptor));
            encode_color_filter(&mut params, filter);
            renderer.dispatch(&output, Some(&layer.pixels), None, &[], &params)?;
            Ok(Some(output))
        }
    }
}

fn resolve_clip<'a>(
    renderer: &VulkanRenderer,
    context: Arc<crate::context::VulkanContext>,
    descriptor: GpuSurfaceDescriptor,
    commands: &GpuCommandBuffer,
    id: Option<GpuClipId>,
    clips: &'a mut HashMap<GpuClipId, PixelBuffer>,
) -> Result<Option<&'a PixelBuffer>, VulkanError> {
    let Some(id) = id else { return Ok(None) };
    ensure_clip(renderer, context, descriptor, commands, id, clips)?;
    Ok(clips.get(&id))
}

fn ensure_clip(
    renderer: &VulkanRenderer,
    context: Arc<crate::context::VulkanContext>,
    descriptor: GpuSurfaceDescriptor,
    commands: &GpuCommandBuffer,
    id: GpuClipId,
    clips: &mut HashMap<GpuClipId, PixelBuffer>,
) -> Result<(), VulkanError> {
    if clips.contains_key(&id) {
        return Ok(());
    }
    let node = commands
        .clip_node(id)
        .ok_or(VulkanError::new(VulkanErrorCode::UnsupportedCommand))?;
    if let Some(parent) = node.parent() {
        ensure_clip(
            renderer,
            context.clone(),
            descriptor,
            commands,
            parent,
            clips,
        )?;
    }
    let edges = match node.geometry() {
        GpuClipGeometry::Rect(rect) => rect_edges(rect, node.transform())?,
        GpuClipGeometry::Path { path, .. } => {
            let path = commands
                .path(path)
                .ok_or(VulkanError::new(VulkanErrorCode::UnsupportedCommand))?;
            path_edges(path, node.transform())?
        }
    };
    let mut params = base_params(OP_CLIP, descriptor);
    params[4] = u32::try_from(edges.len() / 4)
        .map_err(|_| VulkanError::new(VulkanErrorCode::SubmissionFailed))?;
    params[5] = u32::from(matches!(
        node.geometry(),
        GpuClipGeometry::Path {
            rule: FillRule::EvenOdd,
            ..
        }
    ));
    params[6] = u32::from(node.parent().is_some());
    params[7] = u32::from(matches!(node.op(), ClipOp::Difference));
    let mask = PixelBuffer::new(context, descriptor)?;
    let parent = node.parent().and_then(|parent| clips.get(&parent));
    renderer.dispatch(&mask, parent, None, &edges, &params)?;
    clips.insert(id, mask);
    Ok(())
}

fn current_target<'a>(surface: &'a VulkanSurface, layers: &'a [Layer]) -> &'a PixelBuffer {
    layers
        .last()
        .map_or_else(|| surface.pixels(), |layer| &layer.pixels)
}

fn base_params(operation: u32, descriptor: GpuSurfaceDescriptor) -> [u32; PARAMETER_WORDS] {
    let mut params = [0_u32; PARAMETER_WORDS];
    params[0] = operation;
    params[1] = descriptor.width();
    params[2] = descriptor.height();
    params
}

fn draw_params(
    operation: u32,
    descriptor: GpuSurfaceDescriptor,
    paint: &Paint,
    transform: Transform,
    scissor: Option<Rect>,
) -> Result<[u32; PARAMETER_WORDS], VulkanError> {
    let mut params = base_params(operation, descriptor);
    encode_paint(&mut params, paint);
    params[12] = blend_mode(paint.blend_mode());
    set_transform(&mut params, transform)?;
    set_scissor(&mut params, scissor);
    Ok(params)
}

fn set_rect(params: &mut [u32; PARAMETER_WORDS], rect: Rect) {
    params[8] = scalar_word(rect.left());
    params[9] = scalar_word(rect.top());
    params[10] = scalar_word(rect.right());
    params[11] = scalar_word(rect.bottom());
}

fn set_transform(
    params: &mut [u32; PARAMETER_WORDS],
    transform: Transform,
) -> Result<(), VulkanError> {
    let inverse = transform
        .inverse()
        .map_err(|_| VulkanError::new(VulkanErrorCode::UnsupportedCommand))?;
    let origin = inverse
        .map_point(Point::new(Scalar::ZERO, Scalar::ZERO))
        .map_err(|_| VulkanError::new(VulkanErrorCode::SubmissionFailed))?;
    let one =
        Scalar::from_i32(1).map_err(|_| VulkanError::new(VulkanErrorCode::SubmissionFailed))?;
    let x = inverse
        .map_point(Point::new(one, Scalar::ZERO))
        .map_err(|_| VulkanError::new(VulkanErrorCode::SubmissionFailed))?;
    let y = inverse
        .map_point(Point::new(Scalar::ZERO, one))
        .map_err(|_| VulkanError::new(VulkanErrorCode::SubmissionFailed))?;
    params[16] = scalar_delta_word(x.x(), origin.x());
    params[17] = scalar_delta_word(x.y(), origin.y());
    params[18] = scalar_delta_word(y.x(), origin.x());
    params[19] = scalar_delta_word(y.y(), origin.y());
    params[20] = scalar_word(origin.x());
    params[21] = scalar_word(origin.y());
    Ok(())
}

fn set_scissor(params: &mut [u32; PARAMETER_WORDS], scissor: Option<Rect>) {
    let Some(scissor) = scissor else { return };
    params[6] = 1;
    params[24] = scalar_word(scissor.left());
    params[25] = scalar_word(scissor.top());
    params[26] = scalar_word(scissor.right());
    params[27] = scalar_word(scissor.bottom());
}

#[allow(clippy::too_many_arguments)]
fn set_image(
    params: &mut [u32; PARAMETER_WORDS],
    width: u32,
    height: u32,
    source_x: u32,
    source_y: u32,
    source_width: u32,
    source_height: u32,
) {
    params[22] = width;
    params[23] = height;
    params[28] = source_x;
    params[29] = source_y;
    params[30] = source_width;
    params[31] = source_height;
}

fn path_edges(path: &skia_core::Path, transform: Transform) -> Result<Vec<u32>, VulkanError> {
    let limits = FlatteningLimits::for_path(path, DEFAULT_CURVE_STEPS)
        .map_err(|_| VulkanError::new(VulkanErrorCode::SubmissionFailed))?;
    let flattened = PathFlattener::new(limits)
        .flatten(path, transform)
        .map_err(|_| VulkanError::new(VulkanErrorCode::SubmissionFailed))?;
    let mut edges = Vec::new();
    for contour in flattened.contours() {
        let points = contour.points();
        for pair in points.windows(2) {
            push_edge(&mut edges, pair[0], pair[1]);
        }
        if points.len() > 1 && points.first() != points.last() {
            push_edge(&mut edges, points[points.len() - 1], points[0]);
        }
    }
    if edges.is_empty() {
        return Err(VulkanError::new(VulkanErrorCode::UnsupportedCommand));
    }
    Ok(edges)
}

fn rect_edges(rect: Rect, transform: Transform) -> Result<Vec<u32>, VulkanError> {
    let logical = [
        Point::new(rect.left(), rect.top()),
        Point::new(rect.right(), rect.top()),
        Point::new(rect.right(), rect.bottom()),
        Point::new(rect.left(), rect.bottom()),
    ];
    let mut points = [logical[0]; 4];
    for (output, input) in points.iter_mut().zip(logical) {
        *output = transform
            .map_point(input)
            .map_err(|_| VulkanError::new(VulkanErrorCode::SubmissionFailed))?;
    }
    let mut edges = Vec::with_capacity(16);
    for index in 0..4 {
        push_edge(&mut edges, points[index], points[(index + 1) % 4]);
    }
    Ok(edges)
}

fn push_edge(output: &mut Vec<u32>, start: Point, end: Point) {
    output.extend([
        scalar_word(start.x()),
        scalar_word(start.y()),
        scalar_word(end.x()),
        scalar_word(end.y()),
    ]);
}

fn rgba_words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|pixel| u32::from_le_bytes([pixel[0], pixel[1], pixel[2], pixel[3]]))
        .collect()
}

fn full_rect(descriptor: GpuSurfaceDescriptor) -> Rect {
    Rect::new(
        Scalar::ZERO,
        Scalar::ZERO,
        Scalar::from_i32(i32::try_from(descriptor.width()).unwrap_or(i32::MAX))
            .unwrap_or(Scalar::ZERO),
        Scalar::from_i32(i32::try_from(descriptor.height()).unwrap_or(i32::MAX))
            .unwrap_or(Scalar::ZERO),
    )
    .unwrap_or_else(|_| {
        Rect::new(
            Scalar::ZERO,
            Scalar::ZERO,
            Scalar::from_bits(i32::MAX),
            Scalar::from_bits(i32::MAX),
        )
        .expect("positive fallback rect")
    })
}

fn layer_restore_bounds(
    bounds: Option<Rect>,
    transform: Transform,
    descriptor: GpuSurfaceDescriptor,
) -> Result<Rect, VulkanError> {
    let Some(bounds) = bounds else {
        return Ok(full_rect(descriptor));
    };
    let corners = [
        Point::new(bounds.left(), bounds.top()),
        Point::new(bounds.right(), bounds.top()),
        Point::new(bounds.right(), bounds.bottom()),
        Point::new(bounds.left(), bounds.bottom()),
    ];
    let mut left = i32::MAX;
    let mut top = i32::MAX;
    let mut right = i32::MIN;
    let mut bottom = i32::MIN;
    for corner in corners {
        let point = transform
            .map_point(corner)
            .map_err(|_| VulkanError::new(VulkanErrorCode::SubmissionFailed))?;
        left = left.min(point.x().bits());
        top = top.min(point.y().bits());
        right = right.max(point.x().bits());
        bottom = bottom.max(point.y().bits());
    }
    let scale = 1 << 16;
    let floor = |bits: i32| bits.div_euclid(scale);
    let ceil = |bits: i32| bits.saturating_add(scale - 1).div_euclid(scale);
    Rect::new(
        Scalar::from_i32(floor(left))
            .map_err(|_| VulkanError::new(VulkanErrorCode::SubmissionFailed))?,
        Scalar::from_i32(floor(top))
            .map_err(|_| VulkanError::new(VulkanErrorCode::SubmissionFailed))?,
        Scalar::from_i32(ceil(right))
            .map_err(|_| VulkanError::new(VulkanErrorCode::SubmissionFailed))?,
        Scalar::from_i32(ceil(bottom))
            .map_err(|_| VulkanError::new(VulkanErrorCode::SubmissionFailed))?,
    )
    .map_err(|_| VulkanError::new(VulkanErrorCode::SubmissionFailed))
}

fn scalar_word(value: Scalar) -> u32 {
    (value.bits() as f32 / 65_536.0).to_bits()
}

fn scalar_delta_word(value: Scalar, origin: Scalar) -> u32 {
    ((value.bits() - origin.bits()) as f32 / 65_536.0).to_bits()
}

fn color_word(color: Color) -> u32 {
    u32::from_le_bytes(color.channels())
}

fn encode_paint(params: &mut [u32; PARAMETER_WORDS], paint: &Paint) {
    params[3] = color_word(paint.color());
    if let Some(gradient) = paint.gradient() {
        match gradient.geometry() {
            GradientGeometry::Linear { start, end } => {
                params[32] = 1;
                params[35] = scalar_word(start.x());
                params[36] = scalar_word(start.y());
                params[37] = scalar_word(end.x());
                params[38] = scalar_word(end.y());
            }
            GradientGeometry::Radial { center, radius } => {
                params[32] = 2;
                params[35] = scalar_word(center.x());
                params[36] = scalar_word(center.y());
                params[37] = scalar_word(radius);
            }
        }
        params[33] = match gradient.tile_mode() {
            TileMode::Clamp => 0,
            TileMode::Repeat => 1,
            TileMode::Mirror => 2,
        };
        params[34] = u32::try_from(gradient.stops().len()).unwrap_or(0);
        for (index, stop) in gradient.stops().iter().enumerate() {
            params[40 + index] = scalar_word(stop.offset());
            params[56 + index] = color_word(stop.color());
        }
    }
    if let Some(filter) = paint.color_filter() {
        encode_color_filter(params, filter);
    }
}

fn encode_color_filter(params: &mut [u32; PARAMETER_WORDS], filter: ColorFilter) {
    match filter {
        ColorFilter::Matrix(matrix) => {
            params[64] = 1;
            for (index, value) in matrix.values().into_iter().enumerate() {
                params[68 + index] = (value as f32 / 65_536.0).to_bits();
            }
        }
        ColorFilter::Blend { color, mode } => {
            params[64] = 2;
            params[65] = blend_mode(mode);
            params[66] = color_word(color);
        }
    }
}

fn multiply_255(first: u8, second: u8) -> u32 {
    (u32::from(first) * u32::from(second) + 127) / 255
}

fn blend_mode(mode: BlendMode) -> u32 {
    match mode {
        BlendMode::Clear => 0,
        BlendMode::Source => 1,
        BlendMode::Destination => 2,
        BlendMode::SourceOver => 3,
        BlendMode::DestinationOver => 4,
        BlendMode::SourceIn => 5,
        BlendMode::DestinationIn => 6,
        BlendMode::SourceOut => 7,
        BlendMode::DestinationOut => 8,
        BlendMode::SourceAtop => 9,
        BlendMode::DestinationAtop => 10,
        BlendMode::Xor => 11,
        BlendMode::Plus => 12,
        BlendMode::Modulate => 13,
        BlendMode::Multiply => 14,
        BlendMode::Screen => 15,
        BlendMode::Overlay => 16,
        BlendMode::Darken => 17,
        BlendMode::Lighten => 18,
        BlendMode::ColorDodge => 19,
        BlendMode::ColorBurn => 20,
        BlendMode::HardLight => 21,
        BlendMode::SoftLight => 22,
        BlendMode::Difference => 23,
        BlendMode::Exclusion => 24,
        BlendMode::Hue => 25,
        BlendMode::Saturation => 26,
        BlendMode::Color => 27,
        BlendMode::Luminosity => 28,
    }
}
