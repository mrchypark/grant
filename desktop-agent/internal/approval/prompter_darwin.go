//go:build darwin

package approval

/*
#cgo CFLAGS: -x objective-c -fobjc-arc -fblocks
#cgo LDFLAGS: -framework Cocoa

#import <Cocoa/Cocoa.h>
#import <dispatch/dispatch.h>
#include <stdlib.h>
#include <string.h>

static NSArray<NSString *> *certd_split_labels(const char *joined) {
	if (joined == NULL || joined[0] == '\0') {
		return @[];
	}
	NSString *source = [NSString stringWithUTF8String:joined];
	return [source componentsSeparatedByString:@"\x1f"];
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

			NSInteger response = [alert runModal];
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
	"unsafe"

	"github.com/cypark/account/desktop-agent/internal/httpapi"
)

type Prompter struct{}

func NewPrompter() *Prompter {
	return &Prompter{}
}

func (p *Prompter) PromptApproval(_ context.Context, prompt httpapi.ApprovalPrompt) (httpapi.ApprovalDecision, error) {
	if len(prompt.Certificates) == 0 {
		return httpapi.ApprovalDecision{}, fmt.Errorf("no certificates available for approval")
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

	origin := C.CString(prompt.Origin)
	organization := C.CString(prompt.OrganizationID)
	businessNo := C.CString(prompt.BusinessNo)
	action := C.CString(prompt.Action)
	labels := C.CString(strings.Join(joinedLabels, "\x1f"))
	defer C.free(unsafe.Pointer(origin))
	defer C.free(unsafe.Pointer(organization))
	defer C.free(unsafe.Pointer(businessNo))
	defer C.free(unsafe.Pointer(action))
	defer C.free(unsafe.Pointer(labels))

	raw := C.certd_show_approval_modal(origin, organization, businessNo, action, labels, C.int(defaultIndex))
	if raw == nil {
		return httpapi.ApprovalDecision{}, httpapi.ErrApprovalRejected
	}
	defer C.free(unsafe.Pointer(raw))

	packed := C.GoString(raw)
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
