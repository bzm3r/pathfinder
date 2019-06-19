// pathfinder/c/include/pathfinder/pathfinder.h
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#ifndef PF_PATHFINDER_H
#define PF_PATHFINDER_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// Macros

// `canvas`

#define PF_LINE_CAP_BUTT    0
#define PF_LINE_CAP_SQUARE  1
#define PF_LINE_CAP_ROUND   2

// `gl`

#define PF_GL_VERSION_GL3   0
#define PF_GL_VERSION_GLES3 1

// `gpu`

#define PF_CLEAR_FLAGS_HAS_COLOR    0x1
#define PF_CLEAR_FLAGS_HAS_DEPTH    0x2
#define PF_CLEAR_FLAGS_HAS_STENCIL  0x4
#define PF_CLEAR_FLAGS_HAS_RECT     0x8

// Types

// `canvas`

struct PFCanvas;
typedef struct PFCanvas *PFCanvasRef;
struct PFPath;
typedef struct PFPath *PFPathRef;
struct PFCanvasFontContext;
typedef struct PFCanvasFontContext *PFCanvasFontContextRef;
typedef uint8_t PFLineCap;

// `geometry`

struct PFColorF {
    float r, g, b, a;
};
typedef struct PFColorF PFColorF;
struct PFVector2F {
    float x, y;
};
typedef struct PFVector2F PFVector2F;
struct PFVector2I {
    int32_t x, y;
};
typedef struct PFVector2I PFVector2I;
struct PFRectF {
    PFVector2F origin, lower_right;
};
typedef struct PFRectF PFRectF;
struct PFRectI {
    PFVector2I origin, lower_right;
};
typedef struct PFRectI PFRectI;

// `gl`

struct PFGLDevice;
typedef struct PFGLDevice *PFGLDeviceRef;
struct PFGLDestFramebuffer;
typedef struct PFGLDestFramebuffer *PFGLDestFramebufferRef;
typedef const void *(*PFGLFunctionLoader)(const char *data, void *userdata);
struct PFGLRenderer;
typedef struct PFGLRenderer *PFGLRendererRef;
typedef uint32_t PFGLVersion;

// `gpu`

typedef uint8_t PFClearFlags;
struct PFClearParams {
    PFColorF color;
    float depth;
    uint8_t stencil;
    PFRectI rect;
    PFClearFlags flags;
};
typedef struct PFClearParams PFClearParams;
struct PFResourceLoader;
typedef struct PFResourceLoader *PFResourceLoaderRef;

// `renderer`

struct PFRenderOptions {
    uint32_t placeholder;
};
typedef struct PFRenderOptions PFRenderOptions;
struct PFScene;
typedef struct PFScene *PFSceneRef;
struct PFSceneProxy;
typedef struct PFSceneProxy *PFSceneProxyRef;

// Functions

// `canvas`

PFCanvasRef PFCanvasCreate(PFCanvasFontContextRef font_context, const PFVector2F *size);
void PFCanvasDestroy(PFCanvasRef canvas);
PFCanvasFontContextRef PFCanvasFontContextCreate();
void PFCanvasFontContextDestroy(PFCanvasFontContextRef font_context);
PFCanvasFontContextRef PFCanvasFontContextClone(PFCanvasFontContextRef font_context);
PFSceneRef PFCanvasCreateScene(PFCanvasRef canvas);
void PFCanvasFillRect(PFCanvasRef canvas, const PFRectF *rect);
void PFCanvasStrokeRect(PFCanvasRef canvas, const PFRectF *rect);
void PFCanvasSetLineWidth(PFCanvasRef canvas, float new_line_width);
void PFCanvasSetLineCap(PFCanvasRef canvas, PFLineCap new_line_cap);
void PFCanvasFillPath(PFCanvasRef canvas, PFPathRef path);
void PFCanvasStrokePath(PFCanvasRef canvas, PFPathRef path);
PFPathRef PFPathCreate();
void PFPathDestroy(PFPathRef path);
PFPathRef PFPathClone(PFPathRef path);
void PFPathMoveTo(PFPathRef path, const PFVector2F *to);
void PFPathLineTo(PFPathRef path, const PFVector2F *to);
void PFPathQuadraticCurveTo(PFPathRef path, const PFVector2F *ctrl, const PFVector2F *to);
void PFPathBezierCurveTo(PFPathRef path,
                         const PFVector2F *ctrl0,
                         const PFVector2F *ctrl1,
                         const PFVector2F *to);
void PFPathClosePath(PFPathRef path);

// `gl`

PFGLDestFramebufferRef PFGLDestFramebufferCreateFullWindow(const PFVector2I *window_size);
void PFGLDestFramebufferDestroy(PFGLDestFramebufferRef dest_framebuffer);
PFGLDeviceRef PFGLDeviceCreate(PFGLVersion version, uint32_t default_framebuffer);
void PFGLDeviceDestroy(PFGLDeviceRef device);
void PFGLDeviceClear(PFGLDeviceRef device, const PFClearParams *params);
void PFGLLoadWith(PFGLFunctionLoader loader, void *userdata);
PFGLRendererRef PFGLRendererCreate(PFGLDeviceRef device,
                                   PFResourceLoaderRef resources,
                                   PFGLDestFramebufferRef dest_framebuffer);
void PFGLRendererDestroy(PFGLRendererRef renderer);
/// Returns a borrowed reference to the device.
PFGLDeviceRef PFGLRendererGetDevice(PFGLRendererRef renderer);
void PFSceneProxyBuildAndRenderGL(PFSceneProxyRef scene_proxy,
                                  PFGLRendererRef renderer,
                                  const PFRenderOptions *options);

// `gpu`

PFResourceLoaderRef PFFilesystemResourceLoaderLocate();
void PFResourceLoaderDestroy(PFResourceLoaderRef loader);

// `renderer`

PFSceneProxyRef PFSceneProxyCreateFromSceneAndRayonExecutor(PFSceneRef scene);
void PFSceneProxyDestroy(PFSceneProxyRef scene_proxy);

#ifdef __cplusplus
}
#endif

#endif
