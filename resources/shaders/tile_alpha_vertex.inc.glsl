// pathfinder/resources/shaders/tile_alpha_vertex.inc.glsl
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

layout(std140, set = 0, binding = 0) uniform struct UniformInputs {
    vec2 uFramebufferSize;
    vec2 uTileSize;
    vec2 uStencilTextureSize;
    vec2 uViewBoxOrigin;
} uniforms;

layout(location = 0) in vec2 aTessCoord;
layout(location = 1) in uvec3 aTileOrigin;
layout(location = 2) in int aBackdrop;
layout(location = 3) in uint aObject;
layout(location = 4) in uint aTileIndex;

layout(location = 0) out vec2 vTexCoord;
layout(location = 1) out float vBackdrop;
layout(location = 2) out vec4 vColor;

vec4 getFillColor(uint object);

vec2 computeTileOffset(uint tileIndex, float stencilTextureWidth) {
    uint tilesPerRow = uint(stencilTextureWidth / uniforms.uTileSize.x);
    uvec2 tileOffset = uvec2(tileIndex % tilesPerRow, tileIndex / tilesPerRow);
    return vec2(tileOffset) * uniforms.uTileSize;
}

void computeVaryings() {
    vec2 origin = vec2(aTileOrigin.xy) + vec2(aTileOrigin.z & 15u, aTileOrigin.z >> 4u) * 256.0;
    vec2 pixelPosition = (origin + aTessCoord) * uniforms.uTileSize + uniforms.uViewBoxOrigin;
    vec2 position = (pixelPosition / uniforms.uFramebufferSize * 2.0 - 1.0) * vec2(1.0, -1.0);
    vec2 texCoord = computeTileOffset(aTileIndex, uniforms.uStencilTextureSize.x) + aTessCoord * uniforms.uTileSize;

    vTexCoord = texCoord / uniforms.uStencilTextureSize;
    vBackdrop = float(aBackdrop);
    vColor = getFillColor(aObject);
    gl_Position = vec4(position, 0.0, 1.0);
}

