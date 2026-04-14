//go:build darwin

package localization

/*
#cgo CFLAGS: -x objective-c -fobjc-arc
#cgo LDFLAGS: -framework Cocoa

#import <Cocoa/Cocoa.h>
#include <stdlib.h>

static char *repass_preferred_language(void) {
	@autoreleasepool {
		NSArray<NSString *> *languages = [NSLocale preferredLanguages];
		if ([languages count] == 0) {
			return NULL;
		}
		NSString *value = [languages objectAtIndex:0];
		return strdup([value UTF8String]);
	}
}
*/
import "C"

import "unsafe"

func SystemLanguage() Language {
	raw := C.repass_preferred_language()
	if raw == nil {
		return Korean
	}
	defer C.free(unsafe.Pointer(raw))
	return languageFromTag(C.GoString(raw))
}
