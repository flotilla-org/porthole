#import <AVFoundation/AVFoundation.h>
#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>
#import <Foundation/Foundation.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

@interface PortholeRecordAvWriterBox : NSObject
@property(nonatomic, strong) AVAssetWriter *writer;
@property(nonatomic, strong) AVAssetWriterInput *input;
@property(nonatomic, strong) AVAssetWriterInputPixelBufferAdaptor *adaptor;
@property(nonatomic) uint32_t width;
@property(nonatomic) uint32_t height;
@property(nonatomic) uint32_t stride;
@property(nonatomic) BOOL finished;
@end

@implementation PortholeRecordAvWriterBox
@end

typedef struct PortholeRecordAvWriter {
  void *box;
} PortholeRecordAvWriter;

static int porthole_record_set_error(char **out_error, NSString *message) {
  if (out_error != NULL) {
    const char *utf8 = message.UTF8String;
    *out_error = utf8 == NULL ? NULL : strdup(utf8);
  }
  return -1;
}

static int porthole_record_set_nserror(char **out_error, NSString *prefix, NSError *error) {
  if (error == nil) {
    return porthole_record_set_error(out_error, prefix);
  }
  return porthole_record_set_error(out_error, [NSString stringWithFormat:@"%@: %@", prefix, error.localizedDescription]);
}

static PortholeRecordAvWriterBox *porthole_record_box(PortholeRecordAvWriter *writer) {
  if (writer == NULL || writer->box == NULL) {
    return nil;
  }
  return (__bridge PortholeRecordAvWriterBox *)writer->box;
}

int porthole_record_av_writer_create(const char *path,
                                     uint32_t width,
                                     uint32_t height,
                                     uint32_t stride,
                                     PortholeRecordAvWriter **out_writer,
                                     char **out_error) {
  @autoreleasepool {
    if (path == NULL || out_writer == NULL) {
      return porthole_record_set_error(out_error, @"invalid AVAssetWriter create arguments");
    }
    if (width == 0 || height == 0 || (uint64_t)stride < ((uint64_t)width * 4)) {
      return porthole_record_set_error(out_error, @"invalid AVAssetWriter dimensions");
    }

    NSString *pathString = [NSString stringWithUTF8String:path];
    if (pathString == nil) {
      return porthole_record_set_error(out_error, @"record output path is not valid UTF-8");
    }
    if ([[NSFileManager defaultManager] fileExistsAtPath:pathString]) {
      return porthole_record_set_error(out_error, [NSString stringWithFormat:@"record output already exists: %@", pathString]);
    }

    NSURL *url = [NSURL fileURLWithPath:pathString];
    NSError *error = nil;
    AVAssetWriter *writer = [AVAssetWriter assetWriterWithURL:url fileType:AVFileTypeQuickTimeMovie error:&error];
    if (writer == nil) {
      return porthole_record_set_nserror(out_error, @"create AVAssetWriter", error);
    }

    NSDictionary *outputSettings = @{
      AVVideoCodecKey : AVVideoCodecTypeH264,
      AVVideoWidthKey : @(width),
      AVVideoHeightKey : @(height),
    };
    AVAssetWriterInput *input = [AVAssetWriterInput assetWriterInputWithMediaType:AVMediaTypeVideo outputSettings:outputSettings];
    input.expectsMediaDataInRealTime = NO;
    if (![writer canAddInput:input]) {
      return porthole_record_set_error(out_error, @"AVAssetWriter cannot add video input");
    }
    [writer addInput:input];

    NSDictionary *pixelAttributes = @{
      (NSString *)kCVPixelBufferPixelFormatTypeKey : @(kCVPixelFormatType_32BGRA),
      (NSString *)kCVPixelBufferWidthKey : @(width),
      (NSString *)kCVPixelBufferHeightKey : @(height),
    };
    AVAssetWriterInputPixelBufferAdaptor *adaptor =
        [AVAssetWriterInputPixelBufferAdaptor assetWriterInputPixelBufferAdaptorWithAssetWriterInput:input
                                                                           sourcePixelBufferAttributes:pixelAttributes];
    if (![writer startWriting]) {
      return porthole_record_set_nserror(out_error, @"start AVAssetWriter", writer.error);
    }
    [writer startSessionAtSourceTime:kCMTimeZero];

    PortholeRecordAvWriterBox *box = [[PortholeRecordAvWriterBox alloc] init];
    box.writer = writer;
    box.input = input;
    box.adaptor = adaptor;
    box.width = width;
    box.height = height;
    box.stride = stride;
    box.finished = NO;

    PortholeRecordAvWriter *handle = calloc(1, sizeof(PortholeRecordAvWriter));
    if (handle == NULL) {
      [writer cancelWriting];
      return porthole_record_set_error(out_error, @"allocate AVAssetWriter handle");
    }
    handle->box = (__bridge_retained void *)box;
    *out_writer = handle;
    return 0;
  }
}

int porthole_record_av_writer_append(PortholeRecordAvWriter *writer,
                                     uint64_t timestamp_ns,
                                     const unsigned char *bytes,
                                     size_t len,
                                     char **out_error) {
  @autoreleasepool {
    PortholeRecordAvWriterBox *box = porthole_record_box(writer);
    if (box == nil || bytes == NULL) {
      return porthole_record_set_error(out_error, @"invalid AVAssetWriter append arguments");
    }
    if (box.finished) {
      return porthole_record_set_error(out_error, @"cannot append after AVAssetWriter finished");
    }
    size_t required = (size_t)box.stride * (size_t)box.height;
    if (len < required) {
      return porthole_record_set_error(out_error, @"record frame payload is shorter than stride * height");
    }
    for (int attempt = 0; !box.input.readyForMoreMediaData && attempt < 500; attempt++) {
      if (box.writer.status == AVAssetWriterStatusFailed || box.writer.status == AVAssetWriterStatusCancelled) {
        return porthole_record_set_nserror(out_error, @"AVAssetWriter is not writable", box.writer.error);
      }
      usleep(10000);
    }
    if (!box.input.readyForMoreMediaData) {
      return porthole_record_set_error(out_error, @"AVAssetWriter input is not ready for more media data");
    }

    CVPixelBufferRef pixelBuffer = NULL;
    CVReturn cvResult = CVPixelBufferPoolCreatePixelBuffer(kCFAllocatorDefault, box.adaptor.pixelBufferPool, &pixelBuffer);
    if (cvResult != kCVReturnSuccess || pixelBuffer == NULL) {
      return porthole_record_set_error(out_error, @"create AVAssetWriter pixel buffer");
    }

    CVPixelBufferLockBaseAddress(pixelBuffer, 0);
    unsigned char *dst = CVPixelBufferGetBaseAddress(pixelBuffer);
    size_t dstStride = CVPixelBufferGetBytesPerRow(pixelBuffer);
    size_t rowBytes = (size_t)box.width * 4;
    for (uint32_t row = 0; row < box.height; row++) {
      memcpy(dst + (row * dstStride), bytes + ((size_t)row * box.stride), rowBytes);
    }
    CVPixelBufferUnlockBaseAddress(pixelBuffer, 0);

    CMTime presentationTime = CMTimeMake((int64_t)timestamp_ns, 1000000000);
    BOOL appended = [box.adaptor appendPixelBuffer:pixelBuffer withPresentationTime:presentationTime];
    CVPixelBufferRelease(pixelBuffer);
    if (!appended) {
      return porthole_record_set_nserror(out_error, @"append AVAssetWriter pixel buffer", box.writer.error);
    }
    return 0;
  }
}

int porthole_record_av_writer_finish(PortholeRecordAvWriter *writer, char **out_error) {
  @autoreleasepool {
    PortholeRecordAvWriterBox *box = porthole_record_box(writer);
    if (box == nil) {
      return porthole_record_set_error(out_error, @"invalid AVAssetWriter finish arguments");
    }
    if (box.finished) {
      return 0;
    }
    [box.input markAsFinished];
    dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
    [box.writer finishWritingWithCompletionHandler:^{
      dispatch_semaphore_signal(semaphore);
    }];
    dispatch_time_t timeout = dispatch_time(DISPATCH_TIME_NOW, 30 * NSEC_PER_SEC);
    if (dispatch_semaphore_wait(semaphore, timeout) != 0) {
      [box.writer cancelWriting];
      return porthole_record_set_error(out_error, @"finish AVAssetWriter timed out");
    }
    if (box.writer.status != AVAssetWriterStatusCompleted) {
      return porthole_record_set_nserror(out_error, @"finish AVAssetWriter", box.writer.error);
    }
    box.finished = YES;
    return 0;
  }
}

void porthole_record_av_writer_destroy(PortholeRecordAvWriter *writer) {
  @autoreleasepool {
    if (writer == NULL) {
      return;
    }
    PortholeRecordAvWriterBox *box = (__bridge_transfer PortholeRecordAvWriterBox *)writer->box;
    if (box != nil && !box.finished) {
      [box.writer cancelWriting];
    }
    free(writer);
  }
}

void porthole_record_av_writer_free_error(char *error) {
  free(error);
}
