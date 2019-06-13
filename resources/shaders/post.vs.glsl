#version {{version}}

precision highp float;


layout(location = 0) in vec2 aPosition;

layout(location = 0) out vec2 vTexCoord;

void main(){
    vTexCoord = aPosition;
    gl_Position = vec4(aPosition * 2.0 - 1.0, 0.0, 1.0);
}

