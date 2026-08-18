#import <Foundation/Foundation.h>
#import <Sparkle/Sparkle.h>
#include <stdbool.h>

static SPUStandardUpdaterController *OpenMicroUpdaterController;

bool openmicro_sparkle_is_enabled(void) {
    if (![NSThread isMainThread]) {
        NSLog(@"OpenMicro updater configuration must be read on the main thread");
        return false;
    }
    return [NSBundle.mainBundle.infoDictionary[@"OpenMicroAutomaticUpdates"] boolValue];
}

bool openmicro_sparkle_start(void) {
    if (![NSThread isMainThread]) {
        NSLog(@"OpenMicro updater must start on the main thread");
        return false;
    }

    NSDictionary *info = NSBundle.mainBundle.infoDictionary;
    if (!openmicro_sparkle_is_enabled()) {
        return false;
    }

    NSString *feedURL = info[@"SUFeedURL"];
    NSString *publicKey = info[@"SUPublicEDKey"];
    if (![feedURL isKindOfClass:NSString.class] || feedURL.length == 0 ||
        ![publicKey isKindOfClass:NSString.class] || publicKey.length == 0 ||
        ![info[@"SURequireSignedFeed"] boolValue] ||
        ![info[@"SUVerifyUpdateBeforeExtraction"] boolValue]) {
        NSLog(@"OpenMicro updater is disabled because its signed feed is not configured");
        return false;
    }

    if (OpenMicroUpdaterController == nil) {
        OpenMicroUpdaterController = [[SPUStandardUpdaterController alloc]
            initWithStartingUpdater:YES
                 updaterDelegate:nil
              userDriverDelegate:nil];
    }
    return OpenMicroUpdaterController != nil;
}

bool openmicro_sparkle_can_check_for_updates(void) {
    if (![NSThread isMainThread]) {
        NSLog(@"OpenMicro updater checks must run on the main thread");
        return false;
    }
    return OpenMicroUpdaterController != nil &&
           OpenMicroUpdaterController.updater.canCheckForUpdates;
}

bool openmicro_sparkle_session_in_progress(void) {
    if (![NSThread isMainThread]) {
        NSLog(@"OpenMicro updater state must be read on the main thread");
        return false;
    }
    return OpenMicroUpdaterController != nil &&
           OpenMicroUpdaterController.updater.sessionInProgress;
}

bool openmicro_sparkle_check_for_updates(void) {
    if (![NSThread isMainThread]) {
        NSLog(@"OpenMicro updater checks must run on the main thread");
        return false;
    }
    if (OpenMicroUpdaterController == nil ||
        !OpenMicroUpdaterController.updater.canCheckForUpdates) {
        return false;
    }
    [OpenMicroUpdaterController checkForUpdates:nil];
    return true;
}
