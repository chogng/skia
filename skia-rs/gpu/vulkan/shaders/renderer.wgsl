struct Words {
    values: array<u32>,
}

@group(0) @binding(0) var<storage, read_write> output_pixels: Words;
@group(0) @binding(1) var<storage, read> source: Words;
@group(0) @binding(2) var<storage, read> payload: Words;
@group(0) @binding(3) var<storage, read> params: Words;
@group(0) @binding(4) var<storage, read> clip: Words;
@group(0) @binding(5) var shader_image: texture_2d<f32>;
@group(0) @binding(6) var shader_sampler: sampler;

const OP_SOLID: u32 = 1u;
const OP_PATH: u32 = 2u;
const OP_TRIANGLES: u32 = 3u;
const OP_IMAGE: u32 = 4u;
const OP_GLYPH: u32 = 5u;
const OP_LAYER: u32 = 6u;
const OP_CLIP: u32 = 7u;
const OP_BLUR_X: u32 = 8u;
const OP_BLUR_Y: u32 = 9u;
const RUNTIME_SHADER_INSTRUCTION_WORDS: u32 = 6u;
const RUNTIME_SHADER_MAX_INSTRUCTIONS: u32 = 64u;
const RUNTIME_SHADER_HEADER: u32 = 96u;
const RUNTIME_SHADER_INSTRUCTION_BASE: u32 = RUNTIME_SHADER_HEADER + 2u;
const RUNTIME_SHADER_UNIFORM_BASE: u32 =
    RUNTIME_SHADER_INSTRUCTION_BASE + RUNTIME_SHADER_MAX_INSTRUCTIONS * RUNTIME_SHADER_INSTRUCTION_WORDS;

// RUNTIME_SHADER_SPECIALIZATION

fn channel(color: u32, shift: u32) -> u32 {
    return (color >> shift) & 255u;
}

fn pack(r: u32, g: u32, b: u32, a: u32) -> u32 {
    return min(r, 255u) | (min(g, 255u) << 8u) | (min(b, 255u) << 16u) | (min(a, 255u) << 24u);
}

fn srgb_to_linear_channel(channel_value: u32) -> u32 {
    let encoded = f32(channel_value) / 255.0;
    let linear = select(
        pow((encoded + 0.055) / 1.055, 2.4),
        encoded / 12.92,
        encoded <= 0.04045,
    );
    return u32(clamp(round(linear * 255.0), 0.0, 255.0));
}

fn linear_to_srgb_channel(channel_value: u32) -> u32 {
    let linear = f32(channel_value) / 255.0;
    let encoded = select(
        1.055 * pow(linear, 1.0 / 2.4) - 0.055,
        linear * 12.92,
        linear <= 0.0031308,
    );
    return u32(clamp(round(encoded * 255.0), 0.0, 255.0));
}

fn to_linear_srgba(color: u32) -> u32 {
    return pack(
        srgb_to_linear_channel(channel(color, 0u)),
        srgb_to_linear_channel(channel(color, 8u)),
        srgb_to_linear_channel(channel(color, 16u)),
        channel(color, 24u),
    );
}

fn to_encoded_srgba(color: u32) -> u32 {
    return pack(
        linear_to_srgb_channel(channel(color, 0u)),
        linear_to_srgb_channel(channel(color, 8u)),
        linear_to_srgb_channel(channel(color, 16u)),
        channel(color, 24u),
    );
}

fn div255(value: u32) -> u32 {
    return (value + 127u) / 255u;
}

fn canonical(color: u32) -> u32 {
    return select(0u, color, channel(color, 24u) != 0u);
}

fn unpremul(red: u32, green: u32, blue: u32, alpha: u32) -> u32 {
    if alpha == 0u { return 0u; }
    return pack(
        min(255u, (red * 255u + alpha / 2u) / alpha),
        min(255u, (green * 255u + alpha / 2u) / alpha),
        min(255u, (blue * 255u + alpha / 2u) / alpha),
        alpha,
    );
}

fn porter_duff(src: u32, dst: u32, fs: u32, fd: u32) -> u32 {
    let sa = channel(src, 24u);
    let da = channel(dst, 24u);
    let oa = min(255u, div255(sa * fs) + div255(da * fd));
    var out = vec3<u32>();
    for (var index = 0u; index < 3u; index++) {
        let shift = index * 8u;
        let sp = div255(channel(src, shift) * sa);
        let dp = div255(channel(dst, shift) * da);
        out[index] = min(255u, div255(sp * fs) + div255(dp * fd));
    }
    return unpremul(out.x, out.y, out.z, oa);
}

fn separable(src: u32, dst: u32, mode: u32) -> u32 {
    let sa = channel(src, 24u);
    let da = channel(dst, 24u);
    let oa = min(255u, sa + div255(da * (255u - sa)));
    if oa == 0u { return 0u; }
    var out = vec3<u32>();
    for (var index = 0u; index < 3u; index++) {
        let shift = index * 8u;
        let s = channel(src, shift);
        let d = channel(dst, shift);
        var blended = 0u;
        switch mode {
            case 14u: { blended = div255(s * d); }
            case 15u: { blended = s + d - div255(s * d); }
            case 16u: { blended = select(div255(2u * s * d), 255u - div255(2u * (255u - s) * (255u - d)), d > 127u); }
            case 17u: { blended = min(s, d); }
            case 18u: { blended = max(s, d); }
            case 19u: { blended = select(min(255u, d * 255u / max(1u, 255u - s)), 255u, s == 255u); }
            case 20u: { blended = select(255u - min(255u, (255u - d) * 255u / s), 0u, s == 0u); }
            case 21u: { blended = select(div255(2u * s * d), 255u - div255(2u * (255u - s) * (255u - d)), s > 127u); }
            case 22u: {
                if s <= 127u {
                    blended = d - div255(div255((255u - 2u*s) * d) * (255u-d));
                } else {
                    let dark = select(u32(sqrt(f32(d * 255u))), (16u*d*d + 4u*255u*255u - 12u*255u*d) * d / (255u*255u), d <= 63u);
                    blended = d + div255((2u*s - 255u) * (dark-d));
                }
            }
            case 23u: { blended = u32(abs(i32(s) - i32(d))); }
            case 24u: { blended = s + d - div255(2u * s * d); }
            default: { blended = s; }
        }
        let sp = div255(s * sa);
        let dp = div255(d * da);
        let outside_source = div255(sp * (255u - da));
        let outside_destination = div255(dp * (255u - sa));
        let overlap = div255(div255(blended * sa) * da);
        out[index] = min(255u, outside_source + outside_destination + overlap);
    }
    return unpremul(out.x, out.y, out.z, oa);
}

fn luminance(color: vec3<i32>) -> i32 {
    return (77 * color.x + 150 * color.y + 29 * color.z + 128) / 256;
}

fn saturation(color: vec3<i32>) -> i32 {
    return max(color.x, max(color.y, color.z)) - min(color.x, min(color.y, color.z));
}

fn clip_color(color_input: vec3<i32>) -> vec3<i32> {
    var color = color_input;
    let light = luminance(color);
    let low = min(color.x, min(color.y, color.z));
    let high = max(color.x, max(color.y, color.z));
    if low < 0 {
        color = vec3<i32>(light) + (color - vec3<i32>(light)) * light / (light-low);
    }
    if high > 255 {
        color = vec3<i32>(light) + (color - vec3<i32>(light)) * (255-light) / (high-light);
    }
    return clamp(color, vec3<i32>(0), vec3<i32>(255));
}

fn set_luminance(color: vec3<i32>, target_light: i32) -> vec3<i32> {
    return clip_color(color + vec3<i32>(target_light - luminance(color)));
}

fn set_saturation(color: vec3<i32>, target_saturation: i32) -> vec3<i32> {
    let low = min(color.x, min(color.y, color.z));
    let high = max(color.x, max(color.y, color.z));
    if high <= low { return vec3<i32>(0); }
    var result = (color - vec3<i32>(low)) * target_saturation / (high-low);
    if color.x == low { result.x = 0; } else if color.x == high { result.x = target_saturation; }
    if color.y == low { result.y = 0; } else if color.y == high { result.y = target_saturation; }
    if color.z == low { result.z = 0; } else if color.z == high { result.z = target_saturation; }
    return result;
}

fn nonseparable(src: u32, dst: u32, mode: u32) -> u32 {
    let sa = channel(src, 24u);
    let da = channel(dst, 24u);
    let source_color = vec3<i32>(i32(channel(src, 0u)), i32(channel(src, 8u)), i32(channel(src, 16u)));
    let destination_color = vec3<i32>(i32(channel(dst, 0u)), i32(channel(dst, 8u)), i32(channel(dst, 16u)));
    var mixed = destination_color;
    switch mode {
        case 25u: { mixed = set_luminance(set_saturation(source_color, saturation(destination_color)), luminance(destination_color)); }
        case 26u: { mixed = set_luminance(set_saturation(destination_color, saturation(source_color)), luminance(destination_color)); }
        case 27u: { mixed = set_luminance(source_color, luminance(destination_color)); }
        default: { mixed = set_luminance(destination_color, luminance(source_color)); }
    }
    let oa = min(255u, sa + div255(da * (255u-sa)));
    var out = vec3<u32>();
    for (var index = 0u; index < 3u; index++) {
        let sp = div255(u32(source_color[index]) * sa);
        let dp = div255(u32(destination_color[index]) * da);
        out[index] = min(255u, div255(sp*(255u-da)) + div255(dp*(255u-sa)) + div255(div255(u32(mixed[index])*sa)*da));
    }
    return unpremul(out.x, out.y, out.z, oa);
}

fn blend_linear(src: u32, dst: u32, mode: u32) -> u32 {
    let sa = channel(src, 24u);
    let da = channel(dst, 24u);
    switch mode {
        case 0u: { return 0u; }
        case 1u: { return canonical(src); }
        case 2u: { return canonical(dst); }
        case 3u: { return porter_duff(src, dst, 255u, 255u - sa); }
        case 4u: { return porter_duff(src, dst, 255u - da, 255u); }
        case 5u: { return porter_duff(src, dst, da, 0u); }
        case 6u: { return porter_duff(src, dst, 0u, sa); }
        case 7u: { return porter_duff(src, dst, 255u - da, 0u); }
        case 8u: { return porter_duff(src, dst, 0u, 255u - sa); }
        case 9u: { return porter_duff(src, dst, da, 255u - sa); }
        case 10u: { return porter_duff(src, dst, 255u - da, sa); }
        case 11u: { return porter_duff(src, dst, 255u - da, 255u - sa); }
        case 12u: {
            return unpremul(
                min(255u, div255(channel(src, 0u) * sa) + div255(channel(dst, 0u) * da)),
                min(255u, div255(channel(src, 8u) * sa) + div255(channel(dst, 8u) * da)),
                min(255u, div255(channel(src, 16u) * sa) + div255(channel(dst, 16u) * da)),
                min(255u, sa + da),
            );
        }
        case 13u: {
            let oa = div255(sa * da);
            return unpremul(
                div255(div255(channel(src, 0u) * sa) * div255(channel(dst, 0u) * da)),
                div255(div255(channel(src, 8u) * sa) * div255(channel(dst, 8u) * da)),
                div255(div255(channel(src, 16u) * sa) * div255(channel(dst, 16u) * da)),
                oa,
            );
        }
        default: { return select(separable(src, dst, mode), nonseparable(src, dst, mode), mode >= 25u); }
    }
}

fn blend(src: u32, dst: u32, mode: u32) -> u32 {
    let sa = channel(src, 24u);
    let da = channel(dst, 24u);
    if mode == 0u { return 0u; }
    if mode == 1u { return canonical(src); }
    if mode == 2u { return canonical(dst); }
    if mode == 3u && da == 0u { return canonical(src); }
    if mode == 3u && sa == 0u { return canonical(dst); }
    return to_encoded_srgba(blend_linear(to_linear_srgba(src), to_linear_srgba(dst), mode));
}

fn apply_color_filter(color: u32) -> u32 {
    let kind = params.values[64u];
    if kind == 0u { return color; }
    if kind == 2u { return blend(params.values[66u], color, params.values[65u]); }
    let input = vec4<f32>(f32(channel(color, 0u)), f32(channel(color, 8u)), f32(channel(color, 16u)), f32(channel(color, 24u)));
    var result = vec4<u32>();
    for (var row = 0u; row < 4u; row++) {
        let base = 68u + row * 5u;
        let value = bitcast<f32>(params.values[base])*input.x + bitcast<f32>(params.values[base+1u])*input.y + bitcast<f32>(params.values[base+2u])*input.z + bitcast<f32>(params.values[base+3u])*input.w + bitcast<f32>(params.values[base+4u]);
        result[row] = u32(clamp(round(value), 0.0, 255.0));
    }
    return pack(result.x, result.y, result.z, result.w);
}

fn tile_parameter(value: f32, mode: u32) -> f32 {
    if mode == 0u { return clamp(value, 0.0, 1.0); }
    if mode == 1u { return value - floor(value); }
    let repeated = value - floor(value / 2.0) * 2.0;
    return select(repeated, 2.0-repeated, repeated > 1.0);
}

fn runtime_instruction_word(instruction: u32, word: u32) -> u32 {
    if runtime_pipeline_specialized {
        return specialized_runtime_instruction_word(instruction, word);
    }
    return params.values[
        RUNTIME_SHADER_INSTRUCTION_BASE + instruction * RUNTIME_SHADER_INSTRUCTION_WORDS + word
    ];
}

fn runtime_color(value: u32) -> vec4<f32> {
    return vec4<f32>(
        f32(channel(value, 0u)) / 255.0,
        f32(channel(value, 8u)) / 255.0,
        f32(channel(value, 16u)) / 255.0,
        f32(channel(value, 24u)) / 255.0,
    );
}

fn pack_runtime_color(color: vec4<f32>) -> u32 {
    let channels = vec4<u32>(round(clamp(color, vec4<f32>(0.0), vec4<f32>(1.0)) * 255.0));
    return pack(channels.x, channels.y, channels.z, channels.w);
}

fn runtime_coordinate(point: f32, start_bits: u32, end_bits: u32) -> f32 {
    let start = f32(bitcast<i32>(start_bits)) / 65536.0;
    let end = f32(bitcast<i32>(end_bits)) / 65536.0;
    return clamp((point - start) / (end - start), 0.0, 1.0);
}

fn evaluate_runtime_paint(point: vec2<f32>) -> u32 {
    var registers: array<vec4<f32>, 16>;
    var source = vec4<f32>(0.0);
    let count = min(
        select(
            params.values[RUNTIME_SHADER_HEADER],
            runtime_specialized_instruction_count,
            runtime_pipeline_specialized,
        ),
        RUNTIME_SHADER_MAX_INSTRUCTIONS,
    );
    for (var index = 0u; index < count; index++) {
        let opcode = runtime_instruction_word(index, 0u);
        let destination = runtime_instruction_word(index, 1u);
        if opcode == 1u {
            registers[destination] = runtime_color(runtime_instruction_word(index, 2u));
        } else if opcode == 2u {
            registers[destination] = runtime_color(
                params.values[RUNTIME_SHADER_UNIFORM_BASE + runtime_instruction_word(index, 2u)]
            );
        } else if opcode == 3u {
            registers[destination] = vec4<f32>(
                runtime_coordinate(
                    point.x,
                    runtime_instruction_word(index, 2u),
                    runtime_instruction_word(index, 3u),
                )
            );
        } else if opcode == 4u {
            registers[destination] = vec4<f32>(
                runtime_coordinate(
                    point.y,
                    runtime_instruction_word(index, 2u),
                    runtime_instruction_word(index, 3u),
                )
            );
        } else if opcode == 5u {
            registers[destination] = clamp(
                registers[runtime_instruction_word(index, 2u)] + registers[runtime_instruction_word(index, 3u)],
                vec4<f32>(0.0),
                vec4<f32>(1.0),
            );
        } else if opcode == 6u {
            registers[destination] = clamp(
                registers[runtime_instruction_word(index, 2u)] * registers[runtime_instruction_word(index, 3u)],
                vec4<f32>(0.0),
                vec4<f32>(1.0),
            );
        } else if opcode == 7u {
            registers[destination] = mix(
                registers[runtime_instruction_word(index, 2u)],
                registers[runtime_instruction_word(index, 3u)],
                clamp(registers[runtime_instruction_word(index, 4u)].x, 0.0, 1.0),
            );
        } else if opcode == 8u {
            registers[destination] = clamp(
                registers[runtime_instruction_word(index, 2u)],
                vec4<f32>(0.0),
                vec4<f32>(1.0),
            );
        } else if opcode == 9u {
            source = registers[runtime_instruction_word(index, 2u)];
        }
    }
    source.a *= f32(channel(params.values[3u], 24u)) / 255.0;
    return pack_runtime_color(source);
}

fn evaluate_paint(point: vec2<f32>) -> u32 {
    var color = params.values[3u];
    let kind = params.values[32u];
    if kind == 3u {
        return apply_color_filter(evaluate_runtime_paint(point));
    }
    if kind == 4u {
        return apply_color_filter(color);
    }
    if kind != 0u {
        let first = vec2<f32>(bitcast<f32>(params.values[35u]), bitcast<f32>(params.values[36u]));
        var parameter = 0.0;
        if kind == 1u {
            let end = vec2<f32>(bitcast<f32>(params.values[37u]), bitcast<f32>(params.values[38u]));
            let vector = end-first;
            parameter = dot(point-first, vector) / dot(vector, vector);
        } else {
            parameter = distance(point, first) / bitcast<f32>(params.values[37u]);
        }
        parameter = tile_parameter(parameter, params.values[33u]);
        color = params.values[56u];
        let stop_count = params.values[34u];
        for (var index = 1u; index < stop_count; index++) {
            let end_offset = bitcast<f32>(params.values[40u+index]);
            if parameter <= end_offset {
                let start_offset = bitcast<f32>(params.values[39u+index]);
                let amount = select((parameter-start_offset)/(end_offset-start_offset), 1.0, end_offset == start_offset);
                let first_color = params.values[55u+index];
                let second_color = params.values[56u+index];
                var channels = vec4<u32>();
                for (var channel_index = 0u; channel_index < 4u; channel_index++) {
                    let shift = channel_index*8u;
                    channels[channel_index] = u32(mix(f32(channel(first_color, shift)), f32(channel(second_color, shift)), amount) + 0.5);
                }
                color = pack(channels.x, channels.y, channels.z, channels.w);
                break;
            }
            color = params.values[56u+index];
        }
        color = modulate_alpha(color, channel(params.values[3u], 24u));
    }
    return apply_color_filter(color);
}

fn point_in_edges(point: vec2<f32>, offset: u32, count: u32, even_odd: bool) -> bool {
    var winding = 0i;
    var parity = false;
    for (var edge = 0u; edge < count; edge++) {
        let base = offset + edge * 4u;
        let a = vec2<f32>(bitcast<f32>(payload.values[base]), bitcast<f32>(payload.values[base + 1u]));
        let b = vec2<f32>(bitcast<f32>(payload.values[base + 2u]), bitcast<f32>(payload.values[base + 3u]));
        let crosses = (a.y <= point.y && b.y > point.y) || (b.y <= point.y && a.y > point.y);
        if crosses {
            let x = a.x + (point.y - a.y) * (b.x - a.x) / (b.y - a.y);
            if x > point.x {
                parity = !parity;
                winding += select(-1, 1, b.y > a.y);
            }
        }
    }
    return select(winding != 0, parity, even_odd);
}

fn point_in_triangles(point: vec2<f32>, count: u32) -> bool {
    for (var triangle = 0u; triangle < count; triangle++) {
        let base = triangle * 6u;
        let a = vec2<f32>(bitcast<f32>(payload.values[base]), bitcast<f32>(payload.values[base + 1u]));
        let b = vec2<f32>(bitcast<f32>(payload.values[base + 2u]), bitcast<f32>(payload.values[base + 3u]));
        let c = vec2<f32>(bitcast<f32>(payload.values[base + 4u]), bitcast<f32>(payload.values[base + 5u]));
        let ab = (b.x-a.x)*(point.y-a.y) - (b.y-a.y)*(point.x-a.x);
        let bc = (c.x-b.x)*(point.y-b.y) - (c.y-b.y)*(point.x-b.x);
        let ca = (a.x-c.x)*(point.y-c.y) - (a.y-c.y)*(point.x-c.x);
        if (ab >= 0.0 && bc >= 0.0 && ca >= 0.0) || (ab <= 0.0 && bc <= 0.0 && ca <= 0.0) { return true; }
    }
    return false;
}

fn local_point(point: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(
        bitcast<f32>(params.values[16u]) * point.x + bitcast<f32>(params.values[18u]) * point.y + bitcast<f32>(params.values[20u]),
        bitcast<f32>(params.values[17u]) * point.x + bitcast<f32>(params.values[19u]) * point.y + bitcast<f32>(params.values[21u]),
    );
}

fn modulate_alpha(color: u32, opacity: u32) -> u32 {
    return (color & 0x00ffffffu) | (div255(channel(color, 24u) * opacity) << 24u);
}

fn sample_image(point: vec2<f32>) -> u32 {
    let left = bitcast<f32>(params.values[8u]);
    let top = bitcast<f32>(params.values[9u]);
    let right = bitcast<f32>(params.values[10u]);
    let bottom = bitcast<f32>(params.values[11u]);
    let width = params.values[22u];
    let height = params.values[23u];
    let uv = vec2<f32>((point.x-left)/(right-left), (point.y-top)/(bottom-top));
    let source_left = params.values[28u];
    let source_top = params.values[29u];
    let source_width = params.values[30u];
    let source_height = params.values[31u];
    if params.values[13u] == 0u {
        let sx = clamp(source_left + u32(clamp(uv.x * f32(source_width), 0.0, f32(source_width) - 0.001)), 0u, width - 1u);
        let sy = clamp(source_top + u32(clamp(uv.y * f32(source_height), 0.0, f32(source_height) - 0.001)), 0u, height - 1u);
        return payload.values[sy * width + sx];
    }
    let coordinate = uv * vec2<f32>(f32(source_width), f32(source_height)) - vec2<f32>(0.5);
    let base = floor(coordinate);
    let weight = coordinate - base;
    let x0 = u32(clamp(i32(base.x), 0, i32(source_width) - 1)) + source_left;
    let x1 = u32(clamp(i32(base.x) + 1, 0, i32(source_width) - 1)) + source_left;
    let y0 = u32(clamp(i32(base.y), 0, i32(source_height) - 1)) + source_top;
    let y1 = u32(clamp(i32(base.y) + 1, 0, i32(source_height) - 1)) + source_top;
    let colors = array<u32, 4>(payload.values[y0*width+x0], payload.values[y0*width+x1], payload.values[y1*width+x0], payload.values[y1*width+x1]);
    var out = vec4<u32>();
    for (var index = 0u; index < 4u; index++) {
        let shift = index * 8u;
        let top = mix(f32(channel(colors[0], shift)), f32(channel(colors[1], shift)), weight.x);
        let bottom = mix(f32(channel(colors[2], shift)), f32(channel(colors[3], shift)), weight.x);
        out[index] = u32(mix(top, bottom, weight.y) + 0.5);
    }
    return pack(out.x, out.y, out.z, out.w);
}

fn sample_shader_image(point: vec2<f32>) -> u32 {
    let size = textureDimensions(shader_image);
    let coordinate = point / vec2<f32>(f32(size.x), f32(size.y));
    let sample = textureSampleLevel(shader_image, shader_sampler, coordinate, 0.0);
    let channels = vec4<u32>(round(clamp(sample, vec4<f32>(0.0), vec4<f32>(1.0)) * 255.0));
    return pack(channels.x, channels.y, channels.z, channels.w);
}

fn inside_scissor(point: vec2<f32>) -> bool {
    if params.values[6u] == 0u { return true; }
    return point.x >= bitcast<f32>(params.values[24u]) && point.y >= bitcast<f32>(params.values[25u]) && point.x < bitcast<f32>(params.values[26u]) && point.y < bitcast<f32>(params.values[27u]);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let width = params.values[1u];
    let height = params.values[2u];
    if id.x >= width || id.y >= height { return; }
    let index = id.y * width + id.x;
    let point = vec2<f32>(f32(id.x) + 0.5, f32(id.y) + 0.5);
    let operation = params.values[0u];
    if operation == OP_BLUR_X || operation == OP_BLUR_Y {
        let radius = i32(params.values[4u]);
        var sum = vec4<u32>();
        let samples = u32(radius * 2 + 1);
        for (var offset = -radius; offset <= radius; offset++) {
            let x = i32(id.x) + select(0, offset, operation == OP_BLUR_X);
            let y = i32(id.y) + select(0, offset, operation == OP_BLUR_Y);
            if x >= 0 && y >= 0 && x < i32(width) && y < i32(height) {
                let color = source.values[u32(y) * width + u32(x)];
                let alpha = channel(color, 24u);
                if operation == OP_BLUR_X {
                    sum += vec4<u32>(div255(channel(color, 0u)*alpha), div255(channel(color, 8u)*alpha), div255(channel(color, 16u)*alpha), alpha);
                } else {
                    sum += vec4<u32>(channel(color, 0u), channel(color, 8u), channel(color, 16u), alpha);
                }
            }
        }
        let averaged = (sum + vec4<u32>(samples/2u)) / vec4<u32>(samples);
        output_pixels.values[index] = select(pack(averaged.x, averaged.y, averaged.z, averaged.w), unpremul(averaged.x, averaged.y, averaged.z, averaged.w), operation == OP_BLUR_Y);
        return;
    }
    if operation == OP_CLIP {
        let geometry = point_in_edges(point, 0u, params.values[4u], params.values[5u] != 0u);
        let parent = select(true, source.values[index] != 0u, params.values[6u] != 0u);
        output_pixels.values[index] = select(0u, 1u, select(parent && geometry, parent && !geometry, params.values[7u] != 0u));
        return;
    }
    if !inside_scissor(point) || (params.values[7u] != 0u && clip.values[index] == 0u) { return; }
    var covered = false;
    let local = local_point(point);
    var src = evaluate_paint(local);
    if params.values[32u] == 4u {
        src = apply_color_filter(modulate_alpha(
            sample_shader_image(local),
            channel(params.values[3u], 24u),
        ));
    }
    if operation == OP_SOLID {
        covered = local.x >= bitcast<f32>(params.values[8u]) && local.y >= bitcast<f32>(params.values[9u]) && local.x < bitcast<f32>(params.values[10u]) && local.y < bitcast<f32>(params.values[11u]);
    } else if operation == OP_PATH {
        covered = point_in_edges(point, 0u, params.values[4u], params.values[5u] != 0u);
    } else if operation == OP_TRIANGLES {
        covered = point_in_triangles(point, params.values[4u]);
    } else if operation == OP_IMAGE || operation == OP_GLYPH {
        covered = local.x >= bitcast<f32>(params.values[8u]) && local.y >= bitcast<f32>(params.values[9u]) && local.x < bitcast<f32>(params.values[10u]) && local.y < bitcast<f32>(params.values[11u]);
        if covered {
            let sampled = sample_image(local);
            if operation == OP_GLYPH && params.values[5u] != 0u {
                src = modulate_alpha(src, channel(sampled, 24u));
            } else {
                src = apply_color_filter(modulate_alpha(sampled, params.values[4u]));
            }
        }
    } else if operation == OP_LAYER {
        covered = true;
        src = apply_color_filter(modulate_alpha(source.values[index], params.values[4u]));
    }
    if covered {
        output_pixels.values[index] = blend(src, output_pixels.values[index], params.values[12u]);
    }
}
