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
	"context"
	"fmt"
	"strings"
	"sync"
	"unsafe"

	"github.com/cypark/account/desktop-agent/internal/httpapi"
	"github.com/cypark/account/desktop-agent/internal/localization"
)

type Prompter struct{}

type approvalModalInput struct {
	Origin         string
	OrganizationID string
	BusinessNo     string
	Action         string
	Labels         []string
	DefaultIndex   int
	Copy           localization.Strings
}

var resetApprovalModalCancellation = func() {
	C.repass_reset_approval_modal_cancellation()
}

var requestApprovalModalCancellation = func() {
	C.repass_request_cancel_approval_modal()
}

var activateApprovalModalApplication = func() {
	C.repass_activate_approval_app()
}

var prepareApprovalApplicationPolicy = func() int64 {
	return int64(C.repass_prepare_approval_app())
}

var restoreApprovalApplicationPolicy = func(previousPolicy int64) {
	C.repass_restore_approval_app(C.long(previousPolicy))
}

var approvalAppPolicyMu sync.Mutex
var approvalAppPolicyDepth int
var approvalAppPreviousPolicy int64

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

var runApprovalModal = func(input approvalModalInput) (string, bool) {
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
		return "", false
	}
	defer C.free(unsafe.Pointer(raw))
	return C.GoString(raw), true
}

var showApprovalModal = func(input approvalModalInput) (string, bool) {
	restore := prepareApprovalModalApplication()
	defer restore()
	return runApprovalModal(input)
}

func NewPrompter() *Prompter {
	return &Prompter{}
}

func (p *Prompter) PromptApproval(ctx context.Context, prompt httpapi.ApprovalPrompt) (httpapi.ApprovalDecision, error) {
	if len(prompt.Certificates) == 0 {
		return httpapi.ApprovalDecision{}, fmt.Errorf("no certificates available for approval")
	}
	if err := ctx.Err(); err != nil {
		return httpapi.ApprovalDecision{}, err
	}

	joinedLabels := make([]string, 0, len(prompt.Certificates))
	defaultIndex := 0
	for i, cert := range prompt.Certificates {
		label := cert.CommonName
		if strings.TrimSpace(label) == "" {
			label = cert.ID
		}
		joinedLabels = append(joinedLabels, label)
		if cert.BusinessNo == prompt.BusinessNo && cert.BusinessNo != "" {
			defaultIndex = i
		}
	}

	resetApprovalModalCancellation()
	type modalResult struct {
		packed   string
		approved bool
	}
	resultCh := make(chan modalResult, 1)
	go func() {
		copy := localization.ForLanguage(localization.SystemLanguage())
		businessNo := prompt.BusinessNo
		if strings.TrimSpace(businessNo) == "" {
			businessNo = "-"
		}
		packed, approved := showApprovalModal(approvalModalInput{
			Origin:         prompt.Origin,
			OrganizationID: prompt.OrganizationID,
			BusinessNo:     businessNo,
			Action:         prompt.Action,
			Labels:         joinedLabels,
			DefaultIndex:   defaultIndex,
			Copy:           copy,
		})
		resultCh <- modalResult{packed: packed, approved: approved}
	}()

	var result modalResult
	select {
	case <-ctx.Done():
		requestApprovalModalCancellation()
		go func() { <-resultCh }()
		return httpapi.ApprovalDecision{}, ctx.Err()
	case result = <-resultCh:
	}
	if !result.approved {
		return httpapi.ApprovalDecision{}, httpapi.ErrApprovalRejected
	}

	packed := result.packed
	parts := strings.SplitN(packed, "\x1f", 2)
	if len(parts) != 2 {
		return httpapi.ApprovalDecision{}, fmt.Errorf("invalid approval modal response")
	}
	selectedIndex, err := parseSelectedIndex(parts[0], len(prompt.Certificates))
	if err != nil {
		return httpapi.ApprovalDecision{}, err
	}
	password := parts[1]
	if strings.TrimSpace(password) == "" {
		return httpapi.ApprovalDecision{}, httpapi.ErrApprovalRejected
	}

	return httpapi.ApprovalDecision{
		CertificateID: prompt.Certificates[selectedIndex].ID,
		Password:      password,
	}, nil
}

func parseSelectedIndex(value string, max int) (int, error) {
	var selected int
	if _, err := fmt.Sscanf(value, "%d", &selected); err != nil {
		return 0, fmt.Errorf("parse selected certificate index: %w", err)
	}
	if selected < 0 || selected >= max {
		return 0, fmt.Errorf("selected certificate index %d out of range", selected)
	}
	return selected, nil
}
