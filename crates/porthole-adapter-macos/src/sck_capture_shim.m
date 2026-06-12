#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>
#import <Foundation/Foundation.h>
#import <IOSurface/IOSurface.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <dispatch/dispatch.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

typedef struct PortholeSckFrame {
  const uint8_t *data;
  size_t len;
  uint32_t width;
  uint32_t height;
  uint32_t stride;
  uint32_t pixel_format;
  uint64_t timestamp_ns;
} PortholeSckFrame;

typedef void (*PortholeSckFrameCallback)(void *ctx, const PortholeSckFrame *frame);
typedef void (*PortholeSckErrorCallback)(void *ctx, const char *message);

// The native (handle) path: the frame is the CVPixelBuffer's backing
// IOSurface, borrowed for the duration of the callback — the callee retains
// it if it keeps it. No pixel lock, no CPU copy.
typedef struct PortholeSckNativeFrame {
  const void *io_surface; // IOSurfaceRef, borrowed
  uint32_t width;
  uint32_t height;
  uint32_t pixel_format;
  uint64_t timestamp_ns;
} PortholeSckNativeFrame;

typedef void (*PortholeSckNativeFrameCallback)(void *ctx, const PortholeSckNativeFrame *frame);

@interface PortholeSckOutput : NSObject <SCStreamOutput, SCStreamDelegate>
@property(nonatomic, assign) PortholeSckFrameCallback frameCallback;
@property(nonatomic, assign) PortholeSckNativeFrameCallback nativeFrameCallback;
@property(nonatomic, assign) PortholeSckErrorCallback errorCallback;
@property(nonatomic, assign) void *ctx;
@end

@implementation PortholeSckOutput
- (void)stream:(SCStream *)stream didOutputSampleBuffer:(CMSampleBufferRef)sampleBuffer ofType:(SCStreamOutputType)type {
  (void)stream;
  // Frame callbacks are serialized on the sampleHandlerQueue passed to
  // addStreamOutput. porthole_sck_stop clears callbacks via dispatch_sync on
  // that same queue, so this path does not need an additional lock.
  if (type != SCStreamOutputTypeScreen || (self.frameCallback == NULL && self.nativeFrameCallback == NULL)) {
    return;
  }
  if (!CMSampleBufferIsValid(sampleBuffer)) {
    return;
  }
  CVImageBufferRef imageBuffer = CMSampleBufferGetImageBuffer(sampleBuffer);
  if (imageBuffer == NULL) {
    return;
  }
  CVPixelBufferRef pixelBuffer = (CVPixelBufferRef)imageBuffer;

  if (self.nativeFrameCallback != NULL) {
    // Native path: hand over the backing IOSurface; the sample buffer keeps
    // it alive for the duration of the callback.
    IOSurfaceRef surface = CVPixelBufferGetIOSurface(pixelBuffer);
    if (surface == NULL) {
      if (self.errorCallback != NULL) {
        self.errorCallback(self.ctx, "ScreenCaptureKit frame has no backing IOSurface");
      }
      return;
    }
    CMTime nativePts = CMSampleBufferGetPresentationTimeStamp(sampleBuffer);
    uint64_t nativeTimestampNs = 0;
    if (CMTIME_IS_NUMERIC(nativePts) && nativePts.timescale > 0 && nativePts.value >= 0) {
      nativeTimestampNs = (uint64_t)(((__int128)nativePts.value * 1000000000) / nativePts.timescale);
    }
    PortholeSckNativeFrame frame = {
        .io_surface = surface,
        .width = (uint32_t)CVPixelBufferGetWidth(pixelBuffer),
        .height = (uint32_t)CVPixelBufferGetHeight(pixelBuffer),
        .pixel_format = (uint32_t)CVPixelBufferGetPixelFormatType(pixelBuffer),
        .timestamp_ns = nativeTimestampNs,
    };
    self.nativeFrameCallback(self.ctx, &frame);
    return;
  }
  if (CVPixelBufferLockBaseAddress(pixelBuffer, kCVPixelBufferLock_ReadOnly) != kCVReturnSuccess) {
    if (self.errorCallback != NULL) {
      self.errorCallback(self.ctx, "CVPixelBufferLockBaseAddress failed");
    }
    return;
  }

  void *base = CVPixelBufferGetBaseAddress(pixelBuffer);
  size_t width = CVPixelBufferGetWidth(pixelBuffer);
  size_t height = CVPixelBufferGetHeight(pixelBuffer);
  size_t stride = CVPixelBufferGetBytesPerRow(pixelBuffer);
  OSType format = CVPixelBufferGetPixelFormatType(pixelBuffer);
  CMTime pts = CMSampleBufferGetPresentationTimeStamp(sampleBuffer);
  uint64_t timestampNs = 0;
  if (CMTIME_IS_NUMERIC(pts) && pts.timescale > 0 && pts.value >= 0) {
    timestampNs = (uint64_t)(((__int128)pts.value * 1000000000) / pts.timescale);
  }

  if (base != NULL && width <= UINT32_MAX && height <= UINT32_MAX && stride <= UINT32_MAX) {
    PortholeSckFrame frame = {
        .data = (const uint8_t *)base,
        .len = stride * height,
        .width = (uint32_t)width,
        .height = (uint32_t)height,
        .stride = (uint32_t)stride,
        .pixel_format = (uint32_t)format,
        .timestamp_ns = timestampNs,
    };
    self.frameCallback(self.ctx, &frame);
  }

  CVPixelBufferUnlockBaseAddress(pixelBuffer, kCVPixelBufferLock_ReadOnly);
}

- (void)stream:(SCStream *)stream didStopWithError:(NSError *)error {
  (void)stream;
  @synchronized(self) {
    if (self.errorCallback != NULL && self.ctx != NULL && error != nil) {
      self.errorCallback(self.ctx, error.localizedDescription.UTF8String);
    }
  }
}
@end

@interface PortholeSckHandle : NSObject
@property(nonatomic, strong) SCStream *stream;
@property(nonatomic, strong) PortholeSckOutput *output;
@property(nonatomic, strong) dispatch_queue_t queue;
@end

@implementation PortholeSckHandle
@end

static char *porthole_sck_copy_error(NSString *message) {
  const char *utf8 = message.UTF8String;
  if (utf8 == NULL) {
    utf8 = "unknown ScreenCaptureKit error";
  }
  char *copy = malloc(strlen(utf8) + 1);
  if (copy == NULL) {
    abort();
  }
  strcpy(copy, utf8);
  return copy;
}

// Like porthole_sck_copy_error but includes NSError domain+code so the
// caller can disambiguate generic "failed to start stream" messages
// (which SCK uses for several distinct underlying conditions).
static char *porthole_sck_copy_nserror(NSError *error, NSString *context) {
  NSString *full = [NSString stringWithFormat:@"%@: %@ (domain=%@ code=%ld)",
                                                context,
                                                error.localizedDescription ?: @"(no description)",
                                                error.domain ?: @"(no domain)",
                                                (long)error.code];
  return porthole_sck_copy_error(full);
}

// Shared stream setup for the CPU-copy and native (IOSurface) delivery
// paths; `output` arrives with the wanted callback already set.
static char *porthole_sck_start_window_common(uint32_t cgWindowId, PortholeSckOutput *output, void **outHandle) {
  if (@available(macOS 12.3, *)) {
    __block SCShareableContent *content = nil;
    __block NSError *contentError = nil;
    dispatch_semaphore_t contentSem = dispatch_semaphore_create(0);
    [SCShareableContent getShareableContentExcludingDesktopWindows:YES
                                               onScreenWindowsOnly:YES
                                                 completionHandler:^(SCShareableContent *shareableContent, NSError *error) {
                                                   content = shareableContent;
                                                   contentError = error;
                                                   dispatch_semaphore_signal(contentSem);
                                                 }];
    if (dispatch_semaphore_wait(contentSem, dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC)) != 0) {
      return porthole_sck_copy_error(@"timed out waiting for ScreenCaptureKit shareable content");
    }
    if (contentError != nil) {
      return porthole_sck_copy_nserror(contentError, @"SCShareableContent enumeration failed");
    }

    SCWindow *target = nil;
    for (SCWindow *window in content.windows) {
      if (window.windowID == cgWindowId) {
        target = window;
        break;
      }
    }
    if (target == nil) {
      return porthole_sck_copy_error([NSString stringWithFormat:@"no ScreenCaptureKit window for CGWindowID %u", cgWindowId]);
    }

    SCContentFilter *filter = [[SCContentFilter alloc] initWithDesktopIndependentWindow:target];
    CGFloat scale = 1.0;
    CGRect rect = target.frame;
    if (@available(macOS 14.0, *)) {
      // infoForFilter: is documented as 14.0+ but is empirically flaky on
      // 14.0/14.1 and can return nil. Treat nil as "no info available" and
      // fall through to target.frame + scale 1.0 rather than letting an
      // unconditional rect = info.contentRect zero-out the dimensions.
      SCShareableContentInfo *info = [SCShareableContent infoForFilter:filter];
      if (info != nil) {
        if (info.pointPixelScale > 0) {
          scale = info.pointPixelScale;
        }
        rect = info.contentRect;
      }
    }
    size_t width = (size_t)MAX(1.0, ceil(rect.size.width * scale));
    size_t height = (size_t)MAX(1.0, ceil(rect.size.height * scale));

    SCStreamConfiguration *config = [SCStreamConfiguration new];
    config.width = width;
    config.height = height;
    config.pixelFormat = kCVPixelFormatType_32BGRA;
    config.queueDepth = 3;
    config.showsCursor = YES;
    config.capturesAudio = NO;

    SCStream *stream = [[SCStream alloc] initWithFilter:filter configuration:config delegate:output];
    dispatch_queue_t queue = dispatch_queue_create("work.flotilla.porthole.sck-capture", DISPATCH_QUEUE_SERIAL);
    NSError *addError = nil;
    if (![stream addStreamOutput:output type:SCStreamOutputTypeScreen sampleHandlerQueue:queue error:&addError]) {
      return porthole_sck_copy_error(addError.localizedDescription);
    }

    __block NSError *startError = nil;
    dispatch_semaphore_t startSem = dispatch_semaphore_create(0);
    [stream startCaptureWithCompletionHandler:^(NSError *error) {
      startError = error;
      dispatch_semaphore_signal(startSem);
    }];
    if (dispatch_semaphore_wait(startSem, dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC)) != 0) {
      dispatch_sync(queue, ^{
        @synchronized(output) {
          output.frameCallback = NULL;
          output.nativeFrameCallback = NULL;
          output.errorCallback = NULL;
          output.ctx = NULL;
        }
      });
      return porthole_sck_copy_error(@"timed out starting ScreenCaptureKit stream");
    }
    if (startError != nil) {
      return porthole_sck_copy_error(startError.localizedDescription);
    }

    PortholeSckHandle *handle = [PortholeSckHandle new];
    handle.stream = stream;
    handle.output = output;
    handle.queue = queue;
    *outHandle = (__bridge_retained void *)handle;
    return NULL;
  } else {
    return porthole_sck_copy_error(@"ScreenCaptureKit requires macOS 12.3 or newer");
  }
}

char *porthole_sck_start_window(uint32_t cgWindowId,
                                PortholeSckFrameCallback frameCallback,
                                PortholeSckErrorCallback errorCallback,
                                void *ctx,
                                void **outHandle) {
  if (outHandle == NULL) {
    return porthole_sck_copy_error(@"missing outHandle");
  }
  *outHandle = NULL;
  if (frameCallback == NULL) {
    return porthole_sck_copy_error(@"missing frame callback");
  }
  PortholeSckOutput *output = [PortholeSckOutput new];
  output.frameCallback = frameCallback;
  output.errorCallback = errorCallback;
  output.ctx = ctx;
  return porthole_sck_start_window_common(cgWindowId, output, outHandle);
}

char *porthole_sck_start_window_native(uint32_t cgWindowId,
                                       PortholeSckNativeFrameCallback frameCallback,
                                       PortholeSckErrorCallback errorCallback,
                                       void *ctx,
                                       void **outHandle) {
  if (outHandle == NULL) {
    return porthole_sck_copy_error(@"missing outHandle");
  }
  *outHandle = NULL;
  if (frameCallback == NULL) {
    return porthole_sck_copy_error(@"missing native frame callback");
  }
  PortholeSckOutput *output = [PortholeSckOutput new];
  output.nativeFrameCallback = frameCallback;
  output.errorCallback = errorCallback;
  output.ctx = ctx;
  return porthole_sck_start_window_common(cgWindowId, output, outHandle);
}

void porthole_sck_stop(void *rawHandle) {
  if (rawHandle == NULL) {
    return;
  }
  PortholeSckHandle *handle = (__bridge_transfer PortholeSckHandle *)rawHandle;
  if (@available(macOS 12.3, *)) {
    dispatch_sync(handle.queue, ^{
      @synchronized(handle.output) {
        handle.output.frameCallback = NULL;
        handle.output.nativeFrameCallback = NULL;
        handle.output.errorCallback = NULL;
        handle.output.ctx = NULL;
      }
    });
    dispatch_semaphore_t stopSem = dispatch_semaphore_create(0);
    [handle.stream stopCaptureWithCompletionHandler:^(NSError *error) {
      if (error != nil) {
        NSLog(@"Porthole ScreenCaptureKit stop failed: %@", error.localizedDescription);
      }
      dispatch_semaphore_signal(stopSem);
    }];
    dispatch_semaphore_wait(stopSem, dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC));
  }
}

void porthole_sck_free_error(char *message) {
  free(message);
}

// One-shot window screenshot via SCScreenshotManager (macOS 14+). Replaces
// the legacy CGWindowListCreateImage path which returns null on macOS Tahoe
// (26 / Darwin 25) even with Screen Recording granted.
//
// On success: writes a heap-allocated RGBA8 pixel buffer to *outPixels (caller
// must call porthole_sck_free_screenshot to release), its length, dimensions,
// and row stride to the corresponding out-parameters, and returns NULL.
// On failure: returns a heap-allocated UTF-8 error string (caller must call
// porthole_sck_free_error). All out-parameters are left zeroed.
//
// We render the captured CGImage through a CGBitmapContext with
// kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big to normalize the
// pixel format — CGImageGetDataProvider's raw bytes aren't guaranteed
// RGBA across macOS versions, but the bitmap context's output is. Tight
// rowstride (width*4, no padding) makes the buffer suitable for direct
// hand-off to a PNG encoder on the Rust side.
char *porthole_sck_screenshot_window(uint32_t cgWindowId,
                                     uint8_t **outPixels,
                                     size_t *outLen,
                                     uint32_t *outWidth,
                                     uint32_t *outHeight,
                                     uint32_t *outBytesPerRow) {
  if (outPixels == NULL || outLen == NULL || outWidth == NULL || outHeight == NULL || outBytesPerRow == NULL) {
    return porthole_sck_copy_error(@"missing screenshot output pointer");
  }
  *outPixels = NULL;
  *outLen = 0;
  *outWidth = 0;
  *outHeight = 0;
  *outBytesPerRow = 0;

  if (@available(macOS 14.0, *)) {
    __block SCShareableContent *content = nil;
    __block NSError *contentError = nil;
    dispatch_semaphore_t contentSem = dispatch_semaphore_create(0);
    // onScreenWindowsOnly:YES skips minimized windows — they're not in the
    // shareable content set on Tahoe. A caller trying to screenshot a
    // minimized window will surface as "no ScreenCaptureKit window for
    // CGWindowID X" rather than a more specific message; this is by
    // design (you can't screenshot a window that isn't being rendered).
    [SCShareableContent getShareableContentExcludingDesktopWindows:YES
                                               onScreenWindowsOnly:YES
                                                 completionHandler:^(SCShareableContent *shareableContent, NSError *error) {
                                                   content = shareableContent;
                                                   contentError = error;
                                                   dispatch_semaphore_signal(contentSem);
                                                 }];
    if (dispatch_semaphore_wait(contentSem, dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC)) != 0) {
      return porthole_sck_copy_error(@"timed out waiting for ScreenCaptureKit shareable content");
    }
    if (contentError != nil) {
      return porthole_sck_copy_nserror(contentError, @"SCShareableContent enumeration failed");
    }

    SCWindow *target = nil;
    for (SCWindow *window in content.windows) {
      if (window.windowID == cgWindowId) {
        target = window;
        break;
      }
    }
    if (target == nil) {
      return porthole_sck_copy_error([NSString stringWithFormat:@"no ScreenCaptureKit window for CGWindowID %u (window may be minimized or off-screen)", cgWindowId]);
    }

    SCContentFilter *filter = [[SCContentFilter alloc] initWithDesktopIndependentWindow:target];
    CGFloat scale = 1.0;
    CGRect rect = target.frame;
    // infoForFilter: is documented as 14.0+ but is empirically flaky on
    // 14.0/14.1 and can return nil. Treat nil as "no info available" and
    // fall through to target.frame + scale 1.0 rather than letting an
    // unconditional rect = info.contentRect zero-out the dimensions.
    SCShareableContentInfo *info = [SCShareableContent infoForFilter:filter];
    if (info != nil) {
      if (info.pointPixelScale > 0) {
        scale = info.pointPixelScale;
      }
      rect = info.contentRect;
    }
    size_t width = (size_t)MAX(1.0, ceil(rect.size.width * scale));
    size_t height = (size_t)MAX(1.0, ceil(rect.size.height * scale));

    SCStreamConfiguration *config = [SCStreamConfiguration new];
    config.width = width;
    config.height = height;
    config.pixelFormat = kCVPixelFormatType_32BGRA;
    config.showsCursor = NO;
    config.capturesAudio = NO;

    __block CGImageRef capturedImage = NULL;
    __block NSError *captureError = nil;
    dispatch_semaphore_t captureSem = dispatch_semaphore_create(0);
    [SCScreenshotManager captureImageWithFilter:filter
                                  configuration:config
                              completionHandler:^(CGImageRef _Nullable image, NSError *_Nullable error) {
                                if (error != nil) {
                                  captureError = error;
                                } else if (image != NULL) {
                                  capturedImage = CGImageRetain(image);
                                }
                                dispatch_semaphore_signal(captureSem);
                              }];
    if (dispatch_semaphore_wait(captureSem, dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC)) != 0) {
      return porthole_sck_copy_error(@"timed out capturing screenshot via ScreenCaptureKit");
    }
    if (captureError != nil) {
      return porthole_sck_copy_nserror(captureError, @"SCScreenshotManager captureImageWithFilter failed");
    }
    if (capturedImage == NULL) {
      return porthole_sck_copy_error(@"ScreenCaptureKit returned no image");
    }

    size_t imgWidth = CGImageGetWidth(capturedImage);
    size_t imgHeight = CGImageGetHeight(capturedImage);
    if (imgWidth == 0 || imgHeight == 0 || imgWidth > UINT32_MAX || imgHeight > UINT32_MAX) {
      CGImageRelease(capturedImage);
      return porthole_sck_copy_error(@"ScreenCaptureKit returned an image with invalid dimensions");
    }

    size_t bytesPerRow = imgWidth * 4;
    size_t totalBytes = bytesPerRow * imgHeight;
    uint8_t *pixels = malloc(totalBytes);
    if (pixels == NULL) {
      CGImageRelease(capturedImage);
      return porthole_sck_copy_error(@"failed to allocate screenshot pixel buffer");
    }

    CGColorSpaceRef colorSpace = CGColorSpaceCreateDeviceRGB();
    if (colorSpace == NULL) {
      free(pixels);
      CGImageRelease(capturedImage);
      return porthole_sck_copy_error(@"failed to create RGB color space");
    }
    CGContextRef ctx = CGBitmapContextCreate(pixels, imgWidth, imgHeight, 8, bytesPerRow, colorSpace,
                                             kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big);
    CGColorSpaceRelease(colorSpace);
    if (ctx == NULL) {
      free(pixels);
      CGImageRelease(capturedImage);
      return porthole_sck_copy_error(@"failed to create screenshot bitmap context");
    }

    CGContextDrawImage(ctx, CGRectMake(0, 0, imgWidth, imgHeight), capturedImage);
    CGContextRelease(ctx);
    CGImageRelease(capturedImage);

    *outPixels = pixels;
    *outLen = totalBytes;
    *outWidth = (uint32_t)imgWidth;
    *outHeight = (uint32_t)imgHeight;
    *outBytesPerRow = (uint32_t)bytesPerRow;
    return NULL;
  } else {
    return porthole_sck_copy_error(@"SCScreenshotManager requires macOS 14.0 or newer");
  }
}

void porthole_sck_free_screenshot(uint8_t *pixels) {
  free(pixels);
}
