//go:build darwin

package approval

/*
#cgo CFLAGS: -x objective-c -fobjc-arc -fblocks
#cgo LDFLAGS: -framework Cocoa

#import <Cocoa/Cocoa.h>
#import <dispatch/dispatch.h>
#include <stdlib.h>
#include <string.h>

static volatile int repass_should_cancel_modal = 0;
static NSAlert *repass_current_alert = nil;

static NSArray<NSString *> *repass_split_labels(const char *joined) {
	if (joined == NULL || joined[0] == '\0') {
		return @[];
	}
	NSString *source = [NSString stringWithUTF8String:joined];
	return [source componentsSeparatedByString:@"\x1f"];
}

static void repass_reset_approval_modal_cancellation(void) {
	repass_should_cancel_modal = 0;
}

static void repass_request_cancel_approval_modal(void) {
	repass_should_cancel_modal = 1;
	dispatch_async(dispatch_get_main_queue(), ^{
		if (repass_current_alert != nil) {
			[NSApp abortModal];
			[[repass_current_alert window] orderOut:nil];
		}
	});
}

static void repass_activate_approval_app(void) {
	void (^activate)(void) = ^{
		[NSApplication sharedApplication];
		[NSApp activateIgnoringOtherApps:YES];
	};
	if ([NSThread isMainThread]) {
		activate();
	} else {
		dispatch_sync(dispatch_get_main_queue(), activate);
	}
}

static long repass_prepare_approval_app(void) {
	__block long previousPolicy = (long)NSApplicationActivationPolicyAccessory;

	void (^prepare)(void) = ^{
		[NSApplication sharedApplication];
		previousPolicy = (long)[NSApp activationPolicy];
		if ((NSApplicationActivationPolicy)previousPolicy != NSApplicationActivationPolicyRegular) {
			[NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
		}
		[NSApp unhide:nil];
		[NSApp activateIgnoringOtherApps:YES];
	};
	if ([NSThread isMainThread]) {
		prepare();
	} else {
		dispatch_sync(dispatch_get_main_queue(), prepare);
	}
	return previousPolicy;
}

static void repass_restore_approval_app(long previousPolicy) {
	void (^restore)(void) = ^{
		[NSApplication sharedApplication];
		NSApplicationActivationPolicy policy = (NSApplicationActivationPolicy)previousPolicy;
		if ([NSApp activationPolicy] != policy) {
			[NSApp setActivationPolicy:policy];
		}
	};
	if ([NSThread isMainThread]) {
		restore();
	} else {
		dispatch_sync(dispatch_get_main_queue(), restore);
	}
}

static char *repass_show_approval_modal(const char *origin, const char *organization, const char *businessNo, const char *action, const char *labelsJoined, int defaultIndex, const char *title, const char *infoFormat, const char *approveLabel, const char *cancelLabel, const char *certificateLabel, const char *passwordLabel, const char *passwordPlaceholder) {
	__block char *result = NULL;
	__block int didCancel = 0;

	void (^showModal)(void) = ^{
		@autoreleasepool {
			[NSApplication sharedApplication];

			NSArray<NSString *> *labels = repass_split_labels(labelsJoined);
			if ([labels count] == 0) {
				return;
			}

			NSAlert *alert = [[NSAlert alloc] init];
			[alert setAlertStyle:NSAlertStyleWarning];
			[alert setMessageText:[NSString stringWithUTF8String:title]];

			NSString *infoTemplate = [NSString stringWithUTF8String:infoFormat];
			NSString *info = [NSString stringWithFormat:infoTemplate, origin, organization, businessNo, action];
			[alert setInformativeText:info];
			[alert addButtonWithTitle:[NSString stringWithUTF8String:approveLabel]];
			[alert addButtonWithTitle:[NSString stringWithUTF8String:cancelLabel]];

			NSView *accessory = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 360, 116)];

			NSTextField *certLabel = [NSTextField labelWithString:[NSString stringWithUTF8String:certificateLabel]];
			[certLabel setFrame:NSMakeRect(0, 86, 120, 17)];
			[accessory addSubview:certLabel];

			NSPopUpButton *popup = [[NSPopUpButton alloc] initWithFrame:NSMakeRect(0, 54, 360, 28) pullsDown:NO];
			[popup addItemsWithTitles:labels];
			if (defaultIndex >= 0 && defaultIndex < (int)[labels count]) {
				[popup selectItemAtIndex:defaultIndex];
			}
			[accessory addSubview:popup];

			NSTextField *passwordFieldLabel = [NSTextField labelWithString:[NSString stringWithUTF8String:passwordLabel]];
			[passwordFieldLabel setFrame:NSMakeRect(0, 30, 120, 17)];
			[accessory addSubview:passwordFieldLabel];

			NSSecureTextField *passwordField = [[NSSecureTextField alloc] initWithFrame:NSMakeRect(0, 0, 360, 24)];
			[passwordField setPlaceholderString:[NSString stringWithUTF8String:passwordPlaceholder]];
			[accessory addSubview:passwordField];

			[alert setAccessoryView:accessory];
			[[alert window] setInitialFirstResponder:passwordField];
			if (repass_should_cancel_modal) {
				didCancel = 1;
				return;
			}

			repass_current_alert = alert;
			NSInteger response = [alert runModal];
			repass_current_alert = nil;
			if (response != NSAlertFirstButtonReturn) {
				didCancel = 1;
				return;
			}

			NSInteger selected = [popup indexOfSelectedItem];
			NSString *password = [passwordField stringValue];
			NSString *packed = [NSString stringWithFormat:@"%ld\x1f%@", (long)selected, password];
			result = strdup([packed UTF8String]);
		}
	};

	if ([NSThread isMainThread]) {
		showModal();
	} else {
		dispatch_sync(dispatch_get_main_queue(), showModal);
	}

	if (didCancel) {
		return NULL;
	}
	return result;
}
*/
import "C"

import (
	"fmt"
	"strings"
	"sync"
	"unsafe"
)

// darwin 전용 NSApp activation policy 스테이트. 모달이 중첩 진입할 때 최상위
// 진입에서만 policy를 복원하도록 depth를 센다.
var prepareApprovalApplicationPolicy = func() int64 {
	return int64(C.repass_prepare_approval_app())
}

var restoreApprovalApplicationPolicy = func(previousPolicy int64) {
	C.repass_restore_approval_app(C.long(previousPolicy))
}

var (
	approvalAppPolicyMu       sync.Mutex
	approvalAppPolicyDepth    int
	approvalAppPreviousPolicy int64
)

var prepareApprovalModalApplication = func() func() {
	approvalAppPolicyMu.Lock()
	if approvalAppPolicyDepth == 0 {
		approvalAppPreviousPolicy = prepareApprovalApplicationPolicy()
	} else {
		activateApprovalModalApplication()
	}
	approvalAppPolicyDepth++
	approvalAppPolicyMu.Unlock()

	return func() {
		approvalAppPolicyMu.Lock()
		if approvalAppPolicyDepth > 0 {
			approvalAppPolicyDepth--
			if approvalAppPolicyDepth == 0 {
				restoreApprovalApplicationPolicy(approvalAppPreviousPolicy)
			}
		}
		approvalAppPolicyMu.Unlock()
	}
}

func init() {
	resetApprovalModalCancellation = func() {
		C.repass_reset_approval_modal_cancellation()
	}
	requestApprovalModalCancellation = func() {
		C.repass_request_cancel_approval_modal()
	}
	activateApprovalModalApplication = func() {
		C.repass_activate_approval_app()
	}
	runApprovalModal = func(input approvalModalInput) approvalModalOutput {
		origin := C.CString(input.Origin)
		organization := C.CString(input.OrganizationID)
		businessNo := C.CString(input.BusinessNo)
		action := C.CString(input.Action)
		labels := C.CString(strings.Join(input.Labels, "\x1f"))
		title := C.CString(input.Copy.ApprovalTitle)
		infoFormat := C.CString(input.Copy.ApprovalInfoFormat)
		approveLabel := C.CString(input.Copy.Approve)
		cancelLabel := C.CString(input.Copy.Cancel)
		certificateLabel := C.CString(input.Copy.CertificateLabel)
		passwordLabel := C.CString(input.Copy.PasswordLabel)
		passwordPlaceholder := C.CString(input.Copy.PasswordPlaceholder)
		defer C.free(unsafe.Pointer(origin))
		defer C.free(unsafe.Pointer(organization))
		defer C.free(unsafe.Pointer(businessNo))
		defer C.free(unsafe.Pointer(action))
		defer C.free(unsafe.Pointer(labels))
		defer C.free(unsafe.Pointer(title))
		defer C.free(unsafe.Pointer(infoFormat))
		defer C.free(unsafe.Pointer(approveLabel))
		defer C.free(unsafe.Pointer(cancelLabel))
		defer C.free(unsafe.Pointer(certificateLabel))
		defer C.free(unsafe.Pointer(passwordLabel))
		defer C.free(unsafe.Pointer(passwordPlaceholder))

		raw := C.repass_show_approval_modal(
			origin,
			organization,
			businessNo,
			action,
			labels,
			C.int(input.DefaultIndex),
			title,
			infoFormat,
			approveLabel,
			cancelLabel,
			certificateLabel,
			passwordLabel,
			passwordPlaceholder,
		)
		if raw == nil {
			return approvalModalOutput{}
		}
		defer C.free(unsafe.Pointer(raw))
		packed := C.GoString(raw)
		parts := strings.SplitN(packed, "\x1f", 2)
		if len(parts) != 2 {
			return approvalModalOutput{Err: fmt.Errorf("invalid approval modal response")}
		}
		selectedIndex, err := parseSelectedIndex(parts[0], len(input.Labels))
		if err != nil {
			return approvalModalOutput{Err: err}
		}
		return approvalModalOutput{
			SelectedIndex: selectedIndex,
			Password:      []byte(parts[1]),
			Approved:      true,
		}
	}
	// darwin은 모달 표시 전에 NSApp activation policy를 잠시 Regular로 올려야 해서
	// 기본 showApprovalModal(= runApprovalModal 호출)을 prepare/restore로 감싼다.
	showApprovalModal = func(input approvalModalInput) approvalModalOutput {
		restore := prepareApprovalModalApplication()
		defer restore()
		return runApprovalModal(input)
	}
}
