#include <metal_stdlib>
using namespace metal;

struct VertexUniforms {
    float2 scale;
    float2 padding;
    float3x3 texCoordTransform;
};

struct FragmentUniforms {
    float3x3 colorMatrix;
    float3 colorOffset;
    float padding;
    float4 effects;
};

struct VertexOut {
    float4 position [[position]];
    float2 texCoord;
};

vertex VertexOut streamingVertex(
    uint vertexID [[vertex_id]],
    constant VertexUniforms& uniforms [[buffer(0)]]
) {
    constexpr float2 positions[4] = {
        float2(-1.0, -1.0),
        float2(1.0, -1.0),
        float2(-1.0, 1.0),
        float2(1.0, 1.0)
    };
    constexpr float2 texCoords[4] = {
        float2(0.0, 1.0),
        float2(1.0, 1.0),
        float2(0.0, 0.0),
        float2(1.0, 0.0)
    };
    VertexOut out;
    out.position = float4(positions[vertexID] * uniforms.scale, 0.0, 1.0);
    float3 transformed = uniforms.texCoordTransform * float3(texCoords[vertexID], 1.0);
    out.texCoord = transformed.xy;
    return out;
}

float3 adjustedColor(float3 rgb, constant FragmentUniforms& uniforms) {
    float luma = dot(rgb, float3(0.2126, 0.7152, 0.0722));
    rgb = mix(float3(luma), rgb, uniforms.effects.x);
    rgb = (rgb - 0.5) * uniforms.effects.y + 0.5;
    return saturate(rgb * uniforms.effects.z);
}

float sharpenedLuma(texture2d<float> texture, float2 uv, float strength) {
    constexpr sampler s(coord::normalized, address::clamp_to_edge, filter::linear);
    float center = texture.sample(s, uv).r;
    if (strength <= 0.0) return center;
    float2 step = 1.0 / float2(texture.get_width(), texture.get_height());
    float4 neighbors = float4(texture.sample(s, uv + float2(step.x, 0)).r,
        texture.sample(s, uv - float2(step.x, 0)).r,
        texture.sample(s, uv + float2(0, step.y)).r,
        texture.sample(s, uv - float2(0, step.y)).r);
    float lo = min(center, min(min(neighbors.x, neighbors.y), min(neighbors.z, neighbors.w)));
    float hi = max(center, max(max(neighbors.x, neighbors.y), max(neighbors.z, neighbors.w)));
    return clamp(center + (center - dot(neighbors, float4(0.25))) * strength, lo, hi);
}

fragment float4 streamingNV12Fragment(
    VertexOut in [[stage_in]],
    texture2d<float> lumaTexture [[texture(0)]],
    texture2d<float> chromaTexture [[texture(1)]],
    constant FragmentUniforms& uniforms [[buffer(0)]]
) {
    constexpr sampler textureSampler(
        coord::normalized,
        address::clamp_to_edge,
        filter::linear
    );
    float y = sharpenedLuma(lumaTexture, in.texCoord, uniforms.effects.w);
    float2 uv = chromaTexture.sample(textureSampler, in.texCoord).rg;
    float3 yuv = float3(y, uv.x, uv.y) + uniforms.colorOffset;
    float3 rgb = uniforms.colorMatrix * yuv;
    return float4(adjustedColor(rgb, uniforms), 1.0);
}

fragment float4 streamingI420Fragment(
    VertexOut in [[stage_in]],
    texture2d<float> lumaTexture [[texture(0)]],
    texture2d<float> uTexture [[texture(1)]],
    texture2d<float> vTexture [[texture(2)]],
    constant FragmentUniforms& uniforms [[buffer(0)]]
) {
    constexpr sampler textureSampler(
        coord::normalized,
        address::clamp_to_edge,
        filter::linear
    );
    float y = sharpenedLuma(lumaTexture, in.texCoord, uniforms.effects.w);
    float u = uTexture.sample(textureSampler, in.texCoord).r;
    float v = vTexture.sample(textureSampler, in.texCoord).r;
    float3 yuv = float3(y, u, v) + uniforms.colorOffset;
    float3 rgb = uniforms.colorMatrix * yuv;
    return float4(adjustedColor(rgb, uniforms), 1.0);
}

fragment float4 streamingBGRAFragment(
    VertexOut in [[stage_in]],
    texture2d<float> videoTexture [[texture(0)]],
    constant FragmentUniforms& uniforms [[buffer(0)]]
) {
    constexpr sampler textureSampler(
        coord::normalized,
        address::clamp_to_edge,
        filter::linear
    );
    float3 rgb = videoTexture.sample(textureSampler, in.texCoord).rgb;
    if (uniforms.effects.w > 0.0) {
        float2 step = 1.0 / float2(videoTexture.get_width(), videoTexture.get_height());
        float3 a = videoTexture.sample(textureSampler, in.texCoord + float2(step.x, 0)).rgb;
        float3 b = videoTexture.sample(textureSampler, in.texCoord - float2(step.x, 0)).rgb;
        float3 c = videoTexture.sample(textureSampler, in.texCoord + float2(0, step.y)).rgb;
        float3 d = videoTexture.sample(textureSampler, in.texCoord - float2(0, step.y)).rgb;
        float3 lo = min(rgb, min(min(a, b), min(c, d)));
        float3 hi = max(rgb, max(max(a, b), max(c, d)));
        rgb = clamp(rgb + (rgb - (a + b + c + d) * 0.25) * uniforms.effects.w, lo, hi);
    }
    return float4(adjustedColor(rgb, uniforms), 1.0);
}
