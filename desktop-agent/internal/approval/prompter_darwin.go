//go:build darwin

package approval

/*
#cgo CFLAGS: -x objective-c -fobjc-arc -fblocks
#cgo LDFLAGS: -framework Cocoa

#import <Cocoa/Cocoa.h>
#import <dispatch/dispatch.h>
#include <stdlib.h>
#include <string.h>

static volatile int certd_should_cancel_modal = 0;
static NSAlert *certd_current_alert = nil;

static NSArray<NSString *> *certd_split_labels(const char *joined) {
	if (joined == NULL || joined[0] == '\0') {
		return @[];
	}
	NSString *source = [NSString stringWithUTF8String:joined];
	return [source componentsSeparatedByString:@"\x1f"];
}

static void certd_reset_approval_modal_cancellation(void) {
	certd_should_cancel_modal = 0;
}

static void certd_request_cancel_approval_modal(void) {
	certd_should_cancel_modal = 1;
	dispatch_async(dispatch_get_main_queue(), ^{
		if (certd_current_alert != nil) {
			[NSApp abortModal];
			[[certd_current_alert window] orderOut:nil];
		}
	});
}

static void certd_activate_approval_app(void) {
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

static long certd_prepare_approval_app(void) {
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

static void certd_restore_approval_app(long previousPolicy) {
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

static char *certd_show_approval_modal(const char *origin, const char *organization, const char *businessNo, const char *action, const char *labelsJoined, int defaultIndex) {
	__block char *result = NULL;
	__block int didCancel = 0;

	void (^showModal)(void) = ^{
		@autoreleasepool {
			[NSApplication sharedApplication];

			NSArray<NSString *> *labels = certd_split_labels(labelsJoined);
			if ([labels count] == 0) {
				return;
			}

			NSAlert *alert = [[NSAlert alloc] init];
			[alert setAlertStyle:NSAlertStyleWarning];
			[alert setMessageText:@"certd 승인 요청"];

			NSString *info = [NSString stringWithFormat:@"Origin: %s\n조직: %s\n사업자번호: %s\n동작: %s\n\n사용할 인증서를 선택하고 비밀번호를 입력하세요.", origin, organization, businessNo, action];
			[alert setInformativeText:info];
			[alert addButtonWithTitle:@"Approve"];
			[alert addButtonWithTitle:@"Cancel"];

			NSView *accessory = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 360, 116)];

			NSTextField *certLabel = [NSTextField labelWithString:@"공동인증서"];
			[certLabel setFrame:NSMakeRect(0, 86, 120, 17)];
			[accessory addSubview:certLabel];

			NSPopUpButton *popup = [[NSPopUpButton alloc] initWithFrame:NSMakeRect(0, 54, 360, 28) pullsDown:NO];
			[popup addItemsWithTitles:labels];
			if (defaultIndex >= 0 && defaultIndex < (int)[labels count]) {
				[popup selectItemAtIndex:defaultIndex];
			}
			[accessory addSubview:popup];

			NSTextField *passwordLabel = [NSTextField labelWithString:@"비밀번호"];
			[passwordLabel setFrame:NSMakeRect(0, 30, 120, 17)];
			[accessory addSubview:passwordLabel];

			NSSecureTextField *passwordField = [[NSSecureTextField alloc] initWithFrame:NSMakeRect(0, 0, 360, 24)];
			[passwordField setPlaceholderString:@"공동인증서 비밀번호"];
			[accessory addSubview:passwordField];

			[alert setAccessoryView:accessory];
			[[alert window] setInitialFirstResponder:passwordField];
			if (certd_should_cancel_modal) {
				didCancel = 1;
				return;
			}

			certd_current_alert = alert;
			NSInteger response = [alert runModal];
			certd_current_alert = nil;
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
)

type Prompter struct{}

type approvalModalInput struct {
	Origin         string
	OrganizationID string
	BusinessNo     string
	Action         string
	Labels         []string
	DefaultIndex   int
}

var resetApprovalModalCancellation = func() {
	C.certd_reset_approval_modal_cancellation()
}

var requestApprovalModalCancellation = func() {
	C.certd_request_cancel_approval_modal()
}

var activateApprovalModalApplication = func() {
	C.certd_activate_approval_app()
}

var prepareApprovalApplicationPolicy = func() int64 {
	return int64(C.certd_prepare_approval_app())
}

var restoreApprovalApplicationPolicy = func(previousPolicy int64) {
	C.certd_restore_approval_app(C.long(previousPolicy))
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
	defer C.free(unsafe.Pointer(origin))
	defer C.free(unsafe.Pointer(organization))
	defer C.free(unsafe.Pointer(businessNo))
	defer C.free(unsafe.Pointer(action))
	defer C.free(unsafe.Pointer(labels))

	raw := C.certd_show_approval_modal(
		origin,
		organization,
		businessNo,
		action,
		labels,
		C.int(input.DefaultIndex),
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
