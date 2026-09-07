#import <AppKit/AppKit.h>
#include <unistd.h>

@interface LaunchProbe : NSObject <NSApplicationDelegate, NSTextViewDelegate>
@property(strong) NSWindow *window;
@property(strong) NSTextView *text;
@property(copy) NSString *resultPath;
@end
@implementation LaunchProbe
- (void)writeResult {
    NSDictionary *result = @{
        @"pid": @(getpid()),
        @"environment": NSProcessInfo.processInfo.environment[@"PORTHOLE_PROBE_VALUE"] ?: @"",
        @"arguments": NSProcessInfo.processInfo.arguments,
        @"cwd": NSFileManager.defaultManager.currentDirectoryPath,
        @"text": self.text.string ?: @""
    };
    NSData *data = [NSJSONSerialization dataWithJSONObject:result options:0 error:nil];
    [data writeToFile:self.resultPath atomically:YES];
}
- (void)applicationDidFinishLaunching:(NSNotification *)notification {
    (void)notification;
    self.resultPath = NSProcessInfo.processInfo.arguments[1];
    self.window = [[NSWindow alloc] initWithContentRect:NSMakeRect(200, 200, 620, 260)
        styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskResizable
        backing:NSBackingStoreBuffered defer:NO];
    self.window.title = @"Porthole launch correlation probe";
    self.window.releasedWhenClosed = NO;
    self.text = [[NSTextView alloc] initWithFrame:self.window.contentView.bounds];
    self.text.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
    self.text.font = [NSFont systemFontOfSize:24];
    self.text.delegate = self;
    self.window.contentView = self.text;
    [self.window makeKeyAndOrderFront:nil];
    [self.window makeFirstResponder:self.text];
    [NSApp activateIgnoringOtherApps:YES];
    [self writeResult];
}
- (void)textDidChange:(NSNotification *)notification { (void)notification; [self writeResult]; }
- (BOOL)applicationShouldTerminateAfterLastWindowClosed:(NSApplication *)sender { (void)sender; return YES; }
@end
int main(void) {
    @autoreleasepool {
        NSApplication *app = NSApplication.sharedApplication;
        [app setActivationPolicy:NSApplicationActivationPolicyRegular];
        LaunchProbe *delegate = [LaunchProbe new];
        app.delegate = delegate;
        [app run];
    }
    return 0;
}
