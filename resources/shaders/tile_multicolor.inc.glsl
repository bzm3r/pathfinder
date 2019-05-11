// pathfinder/resources/shaders/tile_multicolor.inc.glsl
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

layout(set = 1, binding = 0) uniform sampler2D uFillColorsTexture;
layout(set = 1, binding = 1) uniform struct FillColorsTexture {
    uniform vec2 uFillColorsTexture;
} color_texture;

vec2 computeFillColorTexCoord(uint object, vec2 textureSize) {
    uint width = uint(textureSize.x);
    return (vec2(float(object % width), float(object / width)) + vec2(0.5)) / textureSize;
}

vec4 getFillColor(uint object) {
    vec2 colorTexCoord = computeFillColorTexCoord(object, uFillColorsTextureSize);
    return texture(color_texture.uFillColorsTexture, colorTexCoord);
}
