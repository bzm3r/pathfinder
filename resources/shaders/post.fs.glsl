#version {{version}}

// pathfinder/resources/shaders/post.fs.glsl
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

// TODO(pcwalton): This could be significantly optimized by operating on a
// sparse per-tile basis.

precision highp float;

layout(std140, set = 0, binding = 0) uniform struct UniformInputs {
    vec2 uSourceSize;
    vec4 uFGColor;
    vec4 uBGColor;
    int uGammaCorrectionEnabled;
} uniforms;

layout(std140, set = 1, binding = 0) sampler2D uSource;

in vec2 vTexCoord;

out vec4 oFragColor;

{{{include_post_gamma_correct}}}
{{{include_post_convolve}}}

// Convolve horizontally in this pass.
float sample1Tap(float offset) {
    return texture(uniforms.uSource, vec2(vTexCoord.x + offset, vTexCoord.y)).r;
}

void main() {
    // Apply defringing if necessary.
    vec3 alpha;
    if (uKernel.w == 0.0) {
        alpha = texture(uSource, vTexCoord).rrr;
    } else {
        vec4 alphaLeft, alphaRight;
        float alphaCenter;
        sample9Tap(alphaLeft, alphaCenter, alphaRight, 1.0 / uniforms.uSourceSize.x);

        float r = convolve7Tap(alphaLeft, vec3(alphaCenter, alphaRight.xy));
        float g = convolve7Tap(vec4(alphaLeft.yzw, alphaCenter), alphaRight.xyz);
        float b = convolve7Tap(vec4(alphaLeft.zw, alphaCenter, alphaRight.x), alphaRight.yzw);

        alpha = vec3(r, g, b);
    }

    // Apply gamma correction if necessary.
    if (uniforms.uGammaCorrectionEnabled != 0)
        alpha = gammaCorrect(uniforms.uBGColor.rgb, alpha);

    // Finish.
    oFragColor = vec4(mix(uniforms.uBGColor.rgb, uniforms.uFGColor.rgb, alpha), 1.0);
}
