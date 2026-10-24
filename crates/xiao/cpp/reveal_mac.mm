#import <AppKit/AppKit.h>
#import <Foundation/Foundation.h>

extern "C" bool reveal_path_mac(const char* path) {
    if (!path || !*path) {
        return false;
    }

    @autoreleasepool {
        NSURL* url = [NSURL fileURLWithPath:[NSString stringWithUTF8String:path]];
        if (!url) {
            return false;
        }
        [[NSWorkspace sharedWorkspace] activateFileViewerSelectingURLs:@[ url ]];
        return true;
    }
}
