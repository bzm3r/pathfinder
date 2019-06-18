#version {{version}}

precision highp float;

layout(std140, set = 0, binding = 0) uniform struct UniformStructA {
    vec2 uFramebufferSize;
    vec2 uTileSize;
} us_A;

layout(location = 0) in vec2 aTessCoord;
layout(location = 1) in uint aFromPx;
layout(location = 2) in uint aToPx;
layout(location = 3) in vec2 aFromSubpx;
layout(location = 4) in vec2 aToSubpx;
layout(location = 5) in uint aTileIndex;

layout(location = 0) out vec2 vFrom;
layout(location = 1) out vec2 vTo;

vec2 computeTileOffset(uint tileIndex, float stencilTextureWidth){
    uint tilesPerRow = uint(stencilTextureWidth / uTileSize . x);
    uvec2 tileOffset = uvec2(aTileIndex % tilesPerRow, aTileIndex / tilesPerRow);
    return vec2(tileOffset)* uTileSize;
}

void main(){
    vec2 tileOrigin = computeTileOffset(aTileIndex, uFramebufferSize . x);

    vec2 from = vec2(aFromPx & 15u, aFromPx >> 4u)+ aFromSubpx;
    vec2 to = vec2(aToPx & 15u, aToPx >> 4u)+ aToSubpx;

    vec2 position;
    if(aTessCoord . x < 0.5)
        position . x = floor(min(from . x, to . x));
    else
        position . x = ceil(max(from . x, to . x));
    if(aTessCoord . y < 0.5)
        position . y = floor(min(from . y, to . y));
    else
        position . y = uTileSize . y;

    vFrom = from - position;
    vTo = to - position;

    gl_Position = vec4((tileOrigin + position)/ uFramebufferSize * 2.0 - 1.0, 0.0, 1.0);
}

