// pathfinder/resources/shaders/tile_solid_vertex.inc.glsl
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

layout(std140, set = 0, binding = 0) uniform struct UniformInputs {
    uniform vec2 uFramebufferSize;
    uniform vec2 uTileSize;
    uniform vec2 uViewBoxOrigin;
} uniforms;

layout(location = 0) in vec2 aTessCoord;
layout(location = 1) in vec2 aTileOrigin;
layout(location = 2) in uint aObject;

layout(location = 0) out vec4 vColor;

vec4 getFillColor(uint object);

void computeVaryings() {
    vec2 pixelPosition = (aTileOrigin + aTessCoord) * uniforms.uTileSize + uniforms.uViewBoxOrigin;
    vec2 position = (pixelPosition / uniforms.uFramebufferSize * 2.0 - 1.0) * vec2(1.0, -1.0);

    vColor = getFillColor(aObject);
    gl_Position = vec4(position, 0.0, 1.0);
}
