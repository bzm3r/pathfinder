#version {{version}}

#extension GL_GOOGLE_include_directive : enable

precision highp float;

layout(std140, set = 0, binding = 3) uniform struct UniformStructD {
    vec2 uSourceSize;
    vec4 uFGColor;
    vec4 uBGColor;
    int uGammaCorrectionEnabled;
    vec4 uKernel;
} us_D;

layout(std140, set = 0, binding = 6) sampler2D uSource;

layout(std140, set = 0, binding = 7) uniform sampler2D uGammaLUT;

layout(location = 0) in vec2 vTexCoord;

layout(location = 0) out vec4 oFragColor;

float gammaCorrectChannel(float bgColor, float fgColor){
    return texture(uGammaLUT, vec2(fgColor, 1.0 - bgColor)). r;
}

vec3 gammaCorrect(vec3 bgColor, vec3 fgColor){
    return vec3(gammaCorrectChannel(bgColor . r, fgColor . r),
                gammaCorrectChannel(bgColor . g, fgColor . g),
                gammaCorrectChannel(bgColor . b, fgColor . b));
}

float sample1Tap(float offset);


void sample9Tap(out vec4 outAlphaLeft,
                out float outAlphaCenter,
                out vec4 outAlphaRight,
                float onePixel){
    outAlphaLeft = vec4(uKernel . x > 0.0 ? sample1Tap(- 4.0 * onePixel): 0.0,
                          sample1Tap(- 3.0 * onePixel),
                          sample1Tap(- 2.0 * onePixel),
                          sample1Tap(- 1.0 * onePixel));
    outAlphaCenter = sample1Tap(0.0);
    outAlphaRight = vec4(sample1Tap(1.0 * onePixel),
                          sample1Tap(2.0 * onePixel),
                          sample1Tap(3.0 * onePixel),
                          uKernel . x > 0.0 ? sample1Tap(4.0 * onePixel): 0.0);
}


float convolve7Tap(vec4 alpha0, vec3 alpha1){
    return dot(alpha0, uKernel)+ dot(alpha1, uKernel . zyx);
}



float sample1Tap(float offset){
    return texture(uSource, vec2(vTexCoord . x + offset, vTexCoord . y)). r;
}

void main(){

    vec3 alpha;
    if(uKernel . w == 0.0){
        alpha = texture(uSource, vTexCoord). rrr;
    } else {
        vec4 alphaLeft, alphaRight;
        float alphaCenter;
        sample9Tap(alphaLeft, alphaCenter, alphaRight, 1.0 / uSourceSize . x);

        float r = convolve7Tap(alphaLeft, vec3(alphaCenter, alphaRight . xy));
        float g = convolve7Tap(vec4(alphaLeft . yzw, alphaCenter), alphaRight . xyz);
        float b = convolve7Tap(vec4(alphaLeft . zw, alphaCenter, alphaRight . x), alphaRight . yzw);

        alpha = vec3(r, g, b);
    }


    if(uGammaCorrectionEnabled != 0)
        alpha = gammaCorrect(uBGColor . rgb, alpha);


    oFragColor = vec4(mix(uBGColor . rgb, uFGColor . rgb, alpha), 1.0);
}

