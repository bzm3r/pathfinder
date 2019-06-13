#version {{version}}
precision highp float;

layout(std140, set = 0, binding = 1) uniform struct UniformInputs {
    mat4 uOldTransform;
} uniforms;

layout(std140, set = 0, binding = 2) uniform sampler2D uTexture;

layout(location = 0) in vec2 vTexCoord;

layout(location = 0) out vec4 oFragColor;

void main(){
    vec4 normTexCoord = uOldTransform * vec4(vTexCoord, 0.0, 1.0);
    vec2 texCoord =((normTexCoord . xy / normTexCoord . w)+ 1.0)* 0.5;
    oFragColor = texture(uTexture, texCoord);
}

