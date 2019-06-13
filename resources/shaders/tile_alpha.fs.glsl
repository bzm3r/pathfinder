#version {{version}}

precision highp float;

layout(std140, set = 0, binding = 2) uniform sampler2D uStencilTexture;

layout(location = 0) in vec2 vTexCoord;
layout(location = 1) in float vBackdrop;
layout(location = 2) in vec4 vColor;

layout(location = 0) out vec4 oFragColor;

void main(){
    float coverage = abs(texture(uStencilTexture, vTexCoord). r + vBackdrop);
    oFragColor = vec4(vColor . rgb, vColor . a * coverage);
}

