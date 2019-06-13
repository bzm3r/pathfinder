#version {{version}}

precision highp float;

layout(set = 0, binding = 0) uniform sampler2D uAreaLUT;

layout(location = 0) in vec2 vFrom;
layout(location = 1) in vec2 vTo;

layout(location = 0) out vec4 oFragColor;


void main(){

    vec2 from = vFrom, to = vTo;


    vec2 left = from . x < to . x ? from : to, right = from . x < to . x ? to : from;


    vec2 window = clamp(vec2(from . x, to . x), - 0.5, 0.5);
    float offset = mix(window . x, window . y, 0.5)- left . x;
    float t = offset /(right . x - left . x);


    float y = mix(left . y, right . y, t);
    float d =(right . y - left . y)/(right . x - left . x);


    float dX = window . x - window . y;
    oFragColor = vec4(texture(uAreaLUT, vec2(y + 8.0, abs(d * dX))/ 16.0). r * dX);
}

