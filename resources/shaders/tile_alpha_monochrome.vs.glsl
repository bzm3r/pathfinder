#version {{version}}

#extension GL_GOOGLE_include_directive : enable

precision highp float;

layout(std140, set = 0, binding = 0) uniform struct UniformInputs {
    vec2 uFramebufferSize;
    vec2 uTileSize;
    vec2 uStencilTextureSize;
    vec2 uViewBoxOrigin;
    vec4 uColor;
} uniforms;

layout(location = 0) in vec2 aTessCoord;
layout(location = 1) in uvec3 aTileOrigin;
layout(location = 2) in int aBackdrop;
layout(location = 3) in uint aTileIndex;

layout(location = 0) out vec2 vTexCoord;
layout(location = 1) out float vBackdrop;
layout(location = 2) out vec4 vColor;

vec4 getColor();

vec2 computeTileOffset(uint tileIndex, float stencilTextureWidth){
    uint tilesPerRow = uint(stencilTextureWidth / uTileSize . x);
    uvec2 tileOffset = uvec2(tileIndex % tilesPerRow, tileIndex / tilesPerRow);
    return vec2(tileOffset)* uTileSize;
}

void computeVaryings(){
    vec2 origin = vec2(aTileOrigin . xy)+ vec2(aTileOrigin . z & 15u, aTileOrigin . z >> 4u)* 256.0;
    vec2 pixelPosition =(origin + aTessCoord)* uTileSize + uViewBoxOrigin;
    vec2 position =(pixelPosition / uFramebufferSize * 2.0 - 1.0)* vec2(1.0, - 1.0);
    vec2 maskTexCoordOrigin = computeTileOffset(aTileIndex, uStencilTextureSize . x);
    vec2 maskTexCoord = maskTexCoordOrigin + aTessCoord * uTileSize;

    vTexCoord = maskTexCoord / uStencilTextureSize;
    vBackdrop = float(aBackdrop);
    vColor = getColor();
    gl_Position = vec4(position, 0.0, 1.0);
}

vec4 getColor(){
    return uColor;
}

void main(){
    computeVaryings();
}

