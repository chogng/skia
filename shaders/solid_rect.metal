#include <metal_stdlib>

using namespace metal;

struct SolidVertex {
    float2 position;
};

vertex float4 skia_solid_rect_vertex(
    const device SolidVertex* vertices [[buffer(0)]],
    constant float2& viewport_size [[buffer(1)]],
    uint vertex_id [[vertex_id]]) {
    float2 position = vertices[vertex_id].position;
    return float4(
        position.x / viewport_size.x * 2.0 - 1.0,
        1.0 - position.y / viewport_size.y * 2.0,
        0.0,
        1.0);
}

fragment float4 skia_solid_rect_fragment(
    float4 position [[position]],
    constant float4& color [[buffer(0)]],
    constant uint& has_clip [[buffer(1)]],
    constant uint& has_shape [[buffer(2)]],
    texture2d<float, access::read> clip_mask [[texture(0)]],
    texture2d<float, access::read> shape_mask [[texture(1)]]) {
    if (has_clip != 0 && clip_mask.read(uint2(position.xy)).r < 0.5) {
        discard_fragment();
    }
    if (has_shape != 0 && shape_mask.read(uint2(position.xy)).r < 0.5) {
        discard_fragment();
    }
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
    texture2d<float, access::read> clip_mask [[texture(1)]],
    constant float4& paint [[buffer(0)]],
    constant uint& has_clip [[buffer(1)]]) {
    if (has_clip != 0 && clip_mask.read(uint2(input.position.xy)).r < 0.5) {
        discard_fragment();
    }
    uint2 coordinate = uint2(input.atlas_position);
    float4 sample = atlas.read(coordinate);
    if (input.mask != 0) {
        return float4(paint.rgb, paint.a * sample.a);
    }
    return float4(sample.rgb, sample.a * paint.a);
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
fragment float4 skia_image_fragment(ImageVarying input [[stage_in]], texture2d<float, access::read> image [[texture(0)]], texture2d<float, access::read> clip_mask [[texture(1)]], constant float& opacity [[buffer(0)]], constant uint& has_clip [[buffer(1)]]) {
    if (has_clip != 0 && clip_mask.read(uint2(input.position.xy)).r < 0.5) discard_fragment();
    float4 sample = image.read(uint2(input.image_position));
    return float4(sample.rgb, sample.a * opacity);
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
