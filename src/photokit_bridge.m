#import <Photos/Photos.h>
#import <Foundation/Foundation.h>
#import <AppKit/AppKit.h>

// Status codes matching PhotoAuthStatus in Rust:
// 0: NotDetermined
// 1: Restricted
// 2: Denied
// 3: Authorized
// 4: Limited
int photokit_auth_status(void) {
    if (@available(macOS 11.0, *)) {
        return (int)[PHPhotoLibrary authorizationStatusForAccessLevel:PHAccessLevelReadWrite];
    }
    return (int)[PHPhotoLibrary authorizationStatus];
}

int photokit_request_auth(void) {
    __block int result = 0;
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    if (@available(macOS 11.0, *)) {
        [PHPhotoLibrary requestAuthorizationForAccessLevel:PHAccessLevelReadWrite handler:^(PHAuthorizationStatus status) {
            result = (int)status;
            dispatch_semaphore_signal(sem);
        }];
    } else {
        [PHPhotoLibrary requestAuthorization:^(PHAuthorizationStatus status) {
            result = (int)status;
            dispatch_semaphore_signal(sem);
        }];
    }
    // Wait up to 60 seconds for user response to system TCC prompt
    dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(60 * NSEC_PER_SEC)));
    return result;
}

typedef void (*AssetCallback)(const char* local_id, double created, double modified, int width, int height, void* ctx);

int photokit_fetch_assets(AssetCallback cb, void* ctx) {
    @autoreleasepool {
        int status = photokit_auth_status();
        if (status != 3 && status != 4) {
            return -1; // Not authorized
        }

        PHFetchOptions* options = [[PHFetchOptions alloc] init];
        options.includeHiddenAssets = NO;
        options.includeAllBurstAssets = NO;
        NSSortDescriptor* sort = [NSSortDescriptor sortDescriptorWithKey:@"creationDate" ascending:NO];
        options.sortDescriptors = @[sort];

        PHFetchResult<PHAsset*>* assets = [PHAsset fetchAssetsWithMediaType:PHAssetMediaTypeImage options:options];
        NSUInteger count = assets.count;
        for (NSUInteger i = 0; i < count; i++) {
            PHAsset* asset = [assets objectAtIndex:i];
            const char* local_id = [asset.localIdentifier UTF8String];
            double created = asset.creationDate ? [asset.creationDate timeIntervalSince1970] : 0.0;
            double mod = asset.modificationDate ? [asset.modificationDate timeIntervalSince1970] : 0.0;
            int w = (int)asset.pixelWidth;
            int h = (int)asset.pixelHeight;
            cb(local_id, created, mod, w, h, ctx);
        }
        return (int)count;
    }
}

typedef void (*DataCallback)(const uint8_t* bytes, size_t len, void* ctx);

// Returns:
// 0: Success (cb called with JPEG bytes)
// 1: In iCloud only (not cached locally, skip)
// 2: Error or asset not found
int photokit_get_asset_image_data(const char* local_id, int max_dim, DataCallback cb, void* ctx) {
    @autoreleasepool {
        NSString* ident = [NSString stringWithUTF8String:local_id];
        PHFetchResult<PHAsset*>* result = [PHAsset fetchAssetsWithLocalIdentifiers:@[ident] options:nil];
        if (result.count == 0) return 2;
        PHAsset* asset = result.firstObject;

        PHImageRequestOptions* options = [[PHImageRequestOptions alloc] init];
        options.version = PHImageRequestOptionsVersionCurrent; // edited version
        options.deliveryMode = PHImageRequestOptionsDeliveryModeHighQualityFormat;
        options.synchronous = YES;
        options.networkAccessAllowed = NO; // skip if iCloud only

        __block int status = 2;
        CGSize targetSize = (max_dim > 0) ? CGSizeMake(max_dim, max_dim) : PHImageManagerMaximumSize;

        [[PHImageManager defaultManager] requestImageForAsset:asset
                                                   targetSize:targetSize
                                                  contentMode:PHImageContentModeAspectFit
                                                      options:options
                                                resultHandler:^(NSImage * _Nullable resultImage, NSDictionary * _Nullable info) {
            NSNumber* isInCloud = info[PHImageResultIsInCloudKey];
            if (isInCloud && [isInCloud boolValue]) {
                status = 1; // iCloud only
                return;
            }
            if (resultImage) {
                CGImageRef cgImage = [resultImage CGImageForProposedRect:NULL context:NULL hints:nil];
                if (cgImage) {
                    NSBitmapImageRep *rep = [[NSBitmapImageRep alloc] initWithCGImage:cgImage];
                    NSData *jpegData = [rep representationUsingType:NSBitmapImageFileTypeJPEG properties:@{NSImageCompressionFactor: @(0.85)}];
                    if (jpegData && jpegData.length > 0) {
                        cb((const uint8_t*)jpegData.bytes, jpegData.length, ctx);
                        status = 0;
                    }
                }
            }
        }];
        return status;
    }
}

int photokit_reveal_in_photos(const char* local_id) {
    @autoreleasepool {
        NSString* ident = [NSString stringWithUTF8String:local_id];
        NSString* uuid = [ident componentsSeparatedByString:@"/"].firstObject;
        NSString* scriptText = [NSString stringWithFormat:
            @"tell application \"Photos\"\n"
            @"  activate\n"
            @"  try\n"
            @"    set matched to (every media item whose id starts with \"%@\")\n"
            @"    if (count of matched) > 0 then\n"
            @"      show (first item of matched)\n"
            @"    end if\n"
            @"  end try\n"
            @"end tell", uuid];
        NSAppleScript* script = [[NSAppleScript alloc] initWithSource:scriptText];
        NSDictionary* err = nil;
        [script executeAndReturnError:&err];
        return (err == nil) ? 1 : 0;
    }
}
