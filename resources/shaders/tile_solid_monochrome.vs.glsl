#version {{version}}

#extension GL_GOOGLE_include_directive : enable

precision highp float;

layout(std140, set = 0, binding = 0) uniform struct UniformInputs {
    vec2 uFramebufferSize;
    vec2 uTileSize;
    vec2 uViewBoxOrigin;
} uniforms;

layout(location = 0) in vec2 aTessCoord;
layout(location = 1) in vec2 aTileOrigin;

layout(location = 0) out vec4 vColor;

vec4 getColor();

void computeVaryings(){
    vec2 pixelPosition =(aTileOrigin + aTessCoord)* uTileSize + uViewBoxOrigin;
    vec2 position =(pixelPosition / uFramebufferSize * 2.0 - 1.0)* vec2(1.0, - 1.0);

    vColor = getColor();

    gl_Position = vec4(position, 0.0, 1.0);
}


vec4 getColor(){
    return uColor;
}

void main(){
    computeVaryings();
}

