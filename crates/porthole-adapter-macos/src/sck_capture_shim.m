#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>
#import <Foundation/Foundation.h>
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

@interface PortholeSckOutput : NSObject <SCStreamOutput, SCStreamDelegate>
@property(nonatomic, assign) PortholeSckFrameCallback frameCallback;
@property(nonatomic, assign) PortholeSckErrorCallback errorCallback;
@property(nonatomic, assign) void *ctx;
@end

@implementation PortholeSckOutput
- (void)stream:(SCStream *)stream didOutputSampleBuffer:(CMSampleBufferRef)sampleBuffer ofType:(SCStreamOutputType)type {
  (void)stream;
  if (type != SCStreamOutputTypeScreen || self.frameCallback == NULL) {
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
  if (self.errorCallback != NULL && error != nil) {
    self.errorCallback(self.ctx, error.localizedDescription.UTF8String);
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
      return porthole_sck_copy_error(contentError.localizedDescription);
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
    SCShareableContentInfo *info = [SCShareableContent infoForFilter:filter];
    CGFloat scale = info.pointPixelScale > 0 ? info.pointPixelScale : 1.0;
    CGRect rect = info.contentRect;
    size_t width = (size_t)MAX(1.0, ceil(rect.size.width * scale));
    size_t height = (size_t)MAX(1.0, ceil(rect.size.height * scale));

    SCStreamConfiguration *config = [SCStreamConfiguration new];
    config.width = width;
    config.height = height;
    config.pixelFormat = kCVPixelFormatType_32BGRA;
    config.queueDepth = 3;
    config.showsCursor = YES;
    config.capturesAudio = NO;

    PortholeSckOutput *output = [PortholeSckOutput new];
    output.frameCallback = frameCallback;
    output.errorCallback = errorCallback;
    output.ctx = ctx;

    SCStream *stream = [[SCStream alloc] initWithFilter:filter configuration:config delegate:output];
    dispatch_queue_t queue = dispatch_queue_create("org.flotilla.porthole.sck-capture", DISPATCH_QUEUE_SERIAL);
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

void porthole_sck_stop(void *rawHandle) {
  if (rawHandle == NULL) {
    return;
  }
  PortholeSckHandle *handle = (__bridge_transfer PortholeSckHandle *)rawHandle;
  if (@available(macOS 12.3, *)) {
    dispatch_sync(handle.queue, ^{
      handle.output.frameCallback = NULL;
      handle.output.errorCallback = NULL;
      handle.output.ctx = NULL;
    });
    dispatch_semaphore_t stopSem = dispatch_semaphore_create(0);
    [handle.stream stopCaptureWithCompletionHandler:^(NSError *error) {
      (void)error;
      dispatch_semaphore_signal(stopSem);
    }];
    dispatch_semaphore_wait(stopSem, dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC));
  }
}

void porthole_sck_free_error(char *message) {
  free(message);
}
