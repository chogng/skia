#include <metal_stdlib>

using namespace metal;

struct SolidVertex {
    float2 position;
    float2 local_position;
};

struct SolidVarying {
    float4 position [[position]];
    float2 local_position;
};

struct PaintUniforms {
    float4 color;
    float4 gradient_colors[8];
    float4 gradient_offsets[2];
    float4 gradient_geometry;
    float4 matrix[4];
    float4 matrix_bias;
    float4 filter_color;
    uint4 modes;
    uint4 extra;
};

float4 from_premul(float4 value) {
    if (value.a <= 0.0) return float4(0.0);
    return float4(clamp(value.rgb / value.a, 0.0, 1.0), clamp(value.a, 0.0, 1.0));
}

float luminance(float3 color) {
    return dot(color, float3(77.0 / 256.0, 150.0 / 256.0, 29.0 / 256.0));
}

float saturation(float3 color) {
    return max(color.r, max(color.g, color.b)) - min(color.r, min(color.g, color.b));
}

float3 clip_color(float3 color) {
    float lum = luminance(color);
    float low = min(color.r, min(color.g, color.b));
    float high = max(color.r, max(color.g, color.b));
    if (low < 0.0) color = lum + (color - lum) * lum / max(lum - low, 1.0e-6);
    if (high > 1.0) color = lum + (color - lum) * (1.0 - lum) / max(high - lum, 1.0e-6);
    return clamp(color, 0.0, 1.0);
}

float3 set_luminance(float3 color, float target) {
    return clip_color(color + (target - luminance(color)));
}

float3 set_saturation(float3 color, float target) {
    float low = min(color.r, min(color.g, color.b));
    float high = max(color.r, max(color.g, color.b));
    if (high <= low) return float3(0.0);
    float3 result = (color - low) * target / (high - low);
    result.r = color.r == low ? 0.0 : (color.r == high ? target : result.r);
    result.g = color.g == low ? 0.0 : (color.g == high ? target : result.g);
    result.b = color.b == low ? 0.0 : (color.b == high ? target : result.b);
    return result;
}

float soft_light_channel(float source, float destination) {
    if (source <= 0.5) {
        return destination - (1.0 - 2.0 * source) * destination * (1.0 - destination);
    }
    float curve = destination <= 0.25
        ? ((16.0 * destination - 12.0) * destination + 4.0) * destination
        : sqrt(destination);
    return destination + (2.0 * source - 1.0) * (curve - destination);
}

float3 separable_blend(float3 source, float3 destination, uint mode) {
    if (mode == 14) return source * destination;
    if (mode == 15) return source + destination - source * destination;
    if (mode == 16) return select(2.0 * source * destination, 1.0 - 2.0 * (1.0 - source) * (1.0 - destination), destination > 0.5);
    if (mode == 17) return min(source, destination);
    if (mode == 18) return max(source, destination);
    if (mode == 19) return select(min(destination / max(1.0 - source, 1.0e-6), 1.0), float3(1.0), source >= 1.0);
    if (mode == 20) return select(1.0 - min((1.0 - destination) / max(source, 1.0e-6), 1.0), float3(0.0), source <= 0.0);
    if (mode == 21) return select(2.0 * source * destination, 1.0 - 2.0 * (1.0 - source) * (1.0 - destination), source > 0.5);
    if (mode == 22) return float3(
        soft_light_channel(source.r, destination.r),
        soft_light_channel(source.g, destination.g),
        soft_light_channel(source.b, destination.b));
    if (mode == 23) return abs(source - destination);
    return source + destination - 2.0 * source * destination;
}

float3 nonseparable_blend(float3 source, float3 destination, uint mode) {
    if (mode == 25) return set_luminance(set_saturation(source, saturation(destination)), luminance(destination));
    if (mode == 26) return set_luminance(set_saturation(destination, saturation(source)), luminance(destination));
    if (mode == 27) return set_luminance(source, luminance(destination));
    return set_luminance(destination, luminance(source));
}

float4 composite(float4 source, float4 destination, uint mode) {
    float4 sp = float4(source.rgb * source.a, source.a);
    float4 dp = float4(destination.rgb * destination.a, destination.a);
    if (mode == 0) return float4(0.0);
    if (mode == 1) return source.a <= 0.0 ? float4(0.0) : source;
    if (mode == 2) return destination.a <= 0.0 ? float4(0.0) : destination;
    if (mode == 12) return from_premul(min(sp + dp, 1.0));
    if (mode == 13) return from_premul(sp * dp);
    if (mode <= 11) {
        float sf = 0.0;
        float df = 0.0;
        if (mode == 3) { sf = 1.0; df = 1.0 - source.a; }
        else if (mode == 4) { sf = 1.0 - destination.a; df = 1.0; }
        else if (mode == 5) sf = destination.a;
        else if (mode == 6) df = source.a;
        else if (mode == 7) sf = 1.0 - destination.a;
        else if (mode == 8) df = 1.0 - source.a;
        else if (mode == 9) { sf = destination.a; df = 1.0 - source.a; }
        else if (mode == 10) { sf = 1.0 - destination.a; df = source.a; }
        else if (mode == 11) { sf = 1.0 - destination.a; df = 1.0 - source.a; }
        return from_premul(sp * sf + dp * df);
    }
    float3 blended = mode <= 24
        ? separable_blend(source.rgb, destination.rgb, mode)
        : nonseparable_blend(source.rgb, destination.rgb, mode);
    float alpha = source.a + destination.a * (1.0 - source.a);
    float3 premul = sp.rgb * (1.0 - destination.a)
        + dp.rgb * (1.0 - source.a)
        + blended * source.a * destination.a;
    return from_premul(float4(premul, alpha));
}

float4 apply_filter(float4 source, constant PaintUniforms& paint) {
    if (paint.modes.w == 1) {
        return clamp(float4(
            dot(paint.matrix[0], source),
            dot(paint.matrix[1], source),
            dot(paint.matrix[2], source),
            dot(paint.matrix[3], source)) + paint.matrix_bias, 0.0, 1.0);
    }
    if (paint.modes.w == 2) return composite(paint.filter_color, source, paint.extra.y);
    return source;
}

float gradient_offset(constant PaintUniforms& paint, uint index) {
    return index < 4 ? paint.gradient_offsets[0][index] : paint.gradient_offsets[1][index - 4];
}

float tiled_parameter(float parameter, uint mode) {
    if (mode == 0) return clamp(parameter, 0.0, 1.0);
    if (mode == 1) return fract(parameter);
    float value = fmod(fmod(parameter, 2.0) + 2.0, 2.0);
    return value > 1.0 ? 2.0 - value : value;
}

float4 evaluate_paint(float2 local_position, constant PaintUniforms& paint) {
    float4 source = paint.color;
    if (paint.modes.x != 0) {
        float parameter;
        if (paint.modes.x == 1) {
            float2 start = paint.gradient_geometry.xy;
            float2 vector = paint.gradient_geometry.zw - start;
            parameter = dot(local_position - start, vector) / dot(vector, vector);
        } else {
            parameter = distance(local_position, paint.gradient_geometry.xy) / paint.gradient_geometry.z;
        }
        parameter = tiled_parameter(parameter, paint.modes.z);
        source = paint.gradient_colors[0];
        for (uint index = 1; index < paint.modes.y; ++index) {
            float end = gradient_offset(paint, index);
            if (parameter <= end) {
                float start = gradient_offset(paint, index - 1);
                float amount = end == start ? 1.0 : (parameter - start) / (end - start);
                source = mix(paint.gradient_colors[index - 1], paint.gradient_colors[index], amount);
                break;
            }
            source = paint.gradient_colors[index];
        }
        source.a *= paint.color.a;
    }
    return apply_filter(source, paint);
}

vertex SolidVarying skia_solid_rect_vertex(
    const device SolidVertex* vertices [[buffer(0)]],
    constant float2& viewport_size [[buffer(1)]],
    uint vertex_id [[vertex_id]]) {
    SolidVertex input = vertices[vertex_id];
    SolidVarying output;
    output.position = float4(
        input.position.x / viewport_size.x * 2.0 - 1.0,
        1.0 - input.position.y / viewport_size.y * 2.0,
        0.0,
        1.0);
    output.local_position = input.local_position;
    return output;
}

fragment float4 skia_solid_rect_fragment(
    SolidVarying input [[stage_in]],
    constant PaintUniforms& paint [[buffer(0)]],
    constant uint& has_clip [[buffer(1)]],
    constant uint& has_shape [[buffer(2)]],
    texture2d<float, access::read> clip_mask [[texture(0)]],
    texture2d<float, access::read> shape_mask [[texture(1)]],
    texture2d<float, access::read> destination [[texture(2)]]) {
    uint2 coordinate = uint2(input.position.xy);
    if (has_clip != 0 && clip_mask.read(coordinate).r < 0.5) {
        discard_fragment();
    }
    if (has_shape != 0 && shape_mask.read(coordinate).r < 0.5) {
        discard_fragment();
    }
    return composite(evaluate_paint(input.local_position, paint), destination.read(coordinate), paint.extra.x);
}

struct GlyphVertex {
    float2 position;
    float2 local_position;
    float2 atlas_position;
    uint mask;
};

struct GlyphVarying {
    float4 position [[position]];
    float2 local_position;
    float2 atlas_position;
    uint mask [[flat]];
};

vertex GlyphVarying skia_glyph_vertex(
    const device GlyphVertex* vertices [[buffer(0)]],
    constant float2& viewport_size [[buffer(1)]],
    uint vertex_id [[vertex_id]]) {
    GlyphVertex input = vertices[vertex_id];
    GlyphVarying output;
    output.position = float4(
        input.position.x / viewport_size.x * 2.0 - 1.0,
        1.0 - input.position.y / viewport_size.y * 2.0,
        0.0,
        1.0);
    output.atlas_position = input.atlas_position;
    output.local_position = input.local_position;
    output.mask = input.mask;
    return output;
}

fragment float4 skia_glyph_fragment(
    GlyphVarying input [[stage_in]],
    texture2d<float, access::read> atlas [[texture(0)]],
    texture2d<float, access::read> clip_mask [[texture(1)]],
    texture2d<float, access::read> destination [[texture(2)]],
    constant PaintUniforms& paint [[buffer(0)]],
    constant uint& has_clip [[buffer(1)]]) {
    if (has_clip != 0 && clip_mask.read(uint2(input.position.xy)).r < 0.5) {
        discard_fragment();
    }
    uint2 coordinate = uint2(input.position.xy);
    float4 sample = atlas.read(uint2(input.atlas_position));
    float4 source;
    if (input.mask != 0) {
        source = evaluate_paint(input.local_position, paint);
        source.a *= sample.a;
    } else {
        source = float4(sample.rgb, sample.a * paint.color.a);
        source = apply_filter(source, paint);
    }
    return composite(source, destination.read(coordinate), paint.extra.x);
}

struct ImageVertex { float2 position; float2 image_position; };
struct ImageVarying { float4 position [[position]]; float2 image_position; };
vertex ImageVarying skia_image_vertex(const device ImageVertex* vertices [[buffer(0)]], constant float2& viewport_size [[buffer(1)]], uint vertex_id [[vertex_id]]) {
    ImageVertex input = vertices[vertex_id];
    ImageVarying output;
    output.position = float4(input.position.x / viewport_size.x * 2.0 - 1.0, 1.0 - input.position.y / viewport_size.y * 2.0, 0.0, 1.0);
    output.image_position = input.image_position;
    return output;
}
fragment float4 skia_image_fragment(ImageVarying input [[stage_in]], texture2d<float> image [[texture(0)]], texture2d<float, access::read> clip_mask [[texture(1)]], texture2d<float, access::read> destination [[texture(2)]], constant PaintUniforms& paint [[buffer(0)]], constant uint& has_clip [[buffer(1)]], constant uint& sampling_filter [[buffer(2)]]) {
    if (has_clip != 0 && clip_mask.read(uint2(input.position.xy)).r < 0.5) discard_fragment();
    constexpr sampler nearest_sampler(coord::normalized, address::clamp_to_edge, filter::nearest);
    constexpr sampler linear_sampler(coord::normalized, address::clamp_to_edge, filter::linear);
    float2 image_size = float2(image.get_width(), image.get_height());
    float2 coordinate = input.image_position / image_size;
    float4 sample = sampling_filter == 0
        ? image.sample(nearest_sampler, coordinate)
        : image.sample(linear_sampler, coordinate);
    sample.a *= paint.color.a;
    sample = apply_filter(sample, paint);
    return composite(sample, destination.read(uint2(input.position.xy)), paint.extra.x);
}

vertex float4 skia_filter_vertex(uint vertex_id [[vertex_id]]) {
    const float2 positions[3] = { float2(-1.0, -1.0), float2(3.0, -1.0), float2(-1.0, 3.0) };
    return float4(positions[vertex_id], 0.0, 1.0);
}

fragment float4 skia_box_blur_fragment(
    float4 position [[position]],
    texture2d<float, access::read> source [[texture(0)]],
    constant int2& direction [[buffer(0)]],
    constant uint& radius [[buffer(1)]]) {
    int2 center = int2(position.xy);
    float4 total = float4(0.0);
    for (int offset = -int(radius); offset <= int(radius); ++offset) {
        int2 coordinate = center + direction * offset;
        if (coordinate.x < 0 || coordinate.y < 0 || coordinate.x >= int(source.get_width()) || coordinate.y >= int(source.get_height())) continue;
        float4 sample = source.read(uint2(coordinate));
        total += float4(sample.rgb * sample.a, sample.a);
    }
    total /= float(radius * 2 + 1);
    return from_premul(total);
}

struct ClipEdge {
    float2 start;
    float2 end;
};

struct ClipUniforms {
    uint edge_count;
    uint even_odd;
    uint difference;
    uint has_parent;
};

vertex float4 skia_clip_vertex(uint vertex_id [[vertex_id]]) {
    const float2 positions[3] = {
        float2(-1.0, -1.0),
        float2(3.0, -1.0),
        float2(-1.0, 3.0),
    };
    return float4(positions[vertex_id], 0.0, 1.0);
}

fragment float skia_clip_fragment(
    float4 position [[position]],
    const device ClipEdge* edges [[buffer(0)]],
    constant ClipUniforms& uniforms [[buffer(1)]],
    texture2d<float, access::read> parent [[texture(0)]]) {
    uint2 coordinate = uint2(position.xy);
    bool parent_visible = uniforms.has_parent == 0 || parent.read(coordinate).r >= 0.5;
    if (!parent_visible) {
        return 0.0;
    }

    bool parity = false;
    int winding = 0;
    float2 sample = position.xy;
    for (uint index = 0; index < uniforms.edge_count; ++index) {
        ClipEdge edge = edges[index];
        bool rising = edge.start.y <= sample.y && sample.y < edge.end.y;
        bool falling = edge.end.y <= sample.y && sample.y < edge.start.y;
        if (!rising && !falling) {
            continue;
        }
        float intersection = edge.start.x
            + (sample.y - edge.start.y) * (edge.end.x - edge.start.x)
                / (edge.end.y - edge.start.y);
        if (intersection > sample.x) {
            parity = !parity;
            winding += rising ? 1 : -1;
        }
    }
    bool inside = uniforms.even_odd != 0 ? parity : winding != 0;
    bool visible = uniforms.difference != 0 ? !inside : inside;
    return visible ? 1.0 : 0.0;
}

vertex float4 skia_stroke_vertex(
    const device float2* vertices [[buffer(0)]],
    constant float2& viewport_size [[buffer(1)]],
    uint vertex_id [[vertex_id]]) {
    float2 position = vertices[vertex_id];
    return float4(
        position.x / viewport_size.x * 2.0 - 1.0,
        1.0 - position.y / viewport_size.y * 2.0,
        0.0,
        1.0);
}

fragment float skia_stroke_fragment() {
    return 1.0;
}
