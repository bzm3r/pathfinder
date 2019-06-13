#version {{version}}

precision highp float;

layout(std140, set = 0, binding = 0) uniform struct UniformInputs {
    mat4 uNewTransform;
} uniforms;

layout(location = 0) in vec2 aPosition;

layout(location = 0) out vec2 vTexCoord;

void main(){
    vTexCoord = aPosition;
    gl_Position = uNewTransform * vec4(aPosition, 0.0, 1.0);
}

