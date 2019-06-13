#version {{version}}

precision highp float;

layout(location = 0) in vec4 vColor;

layout(location = 0) out vec4 oFragColor;

void main(){
    oFragColor = vColor;
}

