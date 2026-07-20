#include <metal_stdlib>

using namespace metal;

struct SolidVertex {
    float2 position;
};

vertex float4 skia_solid_rect_vertex(
    const device SolidVertex* vertices [[buffer(0)]],
    uint vertex_id [[vertex_id]]) {
    return float4(vertices[vertex_id].position, 0.0, 1.0);
}

fragment float4 skia_solid_rect_fragment(
    constant float4& color [[buffer(0)]]) {
    return color;
}

struct GlyphVertex {
    float2 position;
    float2 atlas_position;
    uint mask;
};

struct GlyphVarying {
    float4 position [[position]];
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
    output.mask = input.mask;
    return output;
}

fragment float4 skia_glyph_fragment(
    GlyphVarying input [[stage_in]],
    texture2d<float, access::read> atlas [[texture(0)]],
    constant float4& paint [[buffer(0)]]) {
    uint2 coordinate = uint2(input.atlas_position);
    float4 sample = atlas.read(coordinate);
    if (input.mask != 0) {
        return float4(paint.rgb, paint.a * sample.a);
    }
    return float4(sample.rgb, sample.a * paint.a);
}
