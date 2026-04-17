//go:build windows

package approval

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"
	"unicode/utf16"

	"desktop-agent/internal/httpapi"
	"desktop-agent/internal/localization"
	"desktop-agent/internal/npki"
)

func TestPromptApprovalCancelsModalWhenContextIsDone(t *testing.T) {
	originalShow := showApprovalModal
	originalRun := runApprovalModal
	originalActivate := activateApprovalModalApplication
	originalReset := resetApprovalModalCancellation
	originalCancel := requestApprovalModalCancellation
	defer func() {
		showApprovalModal = originalShow
		runApprovalModal = originalRun
		activateApprovalModalApplication = originalActivate
		resetApprovalModalCancellation = originalReset
		requestApprovalModalCancellation = originalCancel
	}()

	started := make(chan struct{})
	release := make(chan struct{})
	canceled := make(chan struct{}, 1)

	showApprovalModal = func(input approvalModalInput) approvalModalOutput {
		close(started)
		<-release
		return approvalModalOutput{}
	}
	resetApprovalModalCancellation = func() {}
	requestApprovalModalCancellation = func() {
		select {
		case canceled <- struct{}{}:
		default:
		}
		close(release)
	}

	ctx, cancel := context.WithCancel(context.Background())
	go func() {
		<-started
		cancel()
	}()

	_, err := NewPrompter().PromptApproval(ctx, httpapi.ApprovalPrompt{
		Origin:         "http://127.0.0.1:4173",
		Action:         "ephemeral",
		OrganizationID: "org-1",
		BusinessNo:     "2768603291",
		Certificates: []npki.CertificateMetadata{{
			ID:         "cert-1",
			CommonName: "Test Certificate",
			BusinessNo: "2768603291",
		}},
	})
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("err = %v, want context.Canceled", err)
	}

	select {
	case <-canceled:
	case <-time.After(2 * time.Second):
		t.Fatal("expected modal cancellation hook to run")
	}
}

func TestPromptApprovalDeliversLocalizedCopyToModal(t *testing.T) {
	originalShow := showApprovalModal
	originalRun := runApprovalModal
	defer func() {
		showApprovalModal = originalShow
		runApprovalModal = originalRun
	}()

	var received approvalModalInput
	runApprovalModal = func(input approvalModalInput) approvalModalOutput {
		received = input
		return approvalModalOutput{SelectedIndex: 0, Password: []byte("secret"), Approved: true}
	}

	_, err := NewPrompter().PromptApproval(context.Background(), httpapi.ApprovalPrompt{
		Origin:         "http://127.0.0.1:4173",
		Action:         "ephemeral",
		OrganizationID: "org-1",
		BusinessNo:     "2768603291",
		Certificates: []npki.CertificateMetadata{{
			ID:         "cert-1",
			CommonName: "Test Certificate",
			BusinessNo: "2768603291",
		}},
	})
	if err != nil {
		t.Fatalf("PromptApproval returned err: %v", err)
	}
	if received.Copy.ApprovalTitle == "" {
		t.Fatal("expected localization copy to be delivered to modal input")
	}
	if received.Copy.Approve == "" || received.Copy.Cancel == "" {
		t.Fatal("expected Approve/Cancel labels to be populated")
	}
	// 시스템 언어에 무관하게 ko 또는 en 중 하나여야 한다.
	ko := localization.ForLanguage(localization.Korean)
	en := localization.ForLanguage(localization.English)
	if received.Copy.ApprovalTitle != ko.ApprovalTitle && received.Copy.ApprovalTitle != en.ApprovalTitle {
		t.Fatalf("unexpected approval title %q", received.Copy.ApprovalTitle)
	}
}

func TestShowWin32ModalActivatesAppBeforeOpeningDialog(t *testing.T) {
	originalActivate := activateApprovalModalApplication
	originalShowDialog := showDialogBoxIndirectParam
	defer func() {
		activateApprovalModalApplication = originalActivate
		showDialogBoxIndirectParam = originalShowDialog
	}()

	events := make([]string, 0, 2)
	activateApprovalModalApplication = func() {
		events = append(events, "activate")
	}
	showDialogBoxIndirectParam = func(template []byte) {
		events = append(events, "dialog")
		currentResult = approvalModalOutput{SelectedIndex: 0, Password: []byte("secret"), Approved: true}
	}

	result := showWin32Modal(approvalModalInput{
		Origin:         "http://127.0.0.1:4173",
		OrganizationID: "org-1",
		BusinessNo:     "2768603291",
		Action:         "ephemeral",
		Labels:         []string{"Test Certificate"},
		DefaultIndex:   0,
		Copy:           localization.ForLanguage(localization.English),
	})
	if !result.Approved {
		t.Fatal("showWin32Modal returned Approved=false, want true")
	}
	if string(result.Password) != "secret" || result.SelectedIndex != 0 {
		t.Fatalf("result = %#v, want selected index 0 with password secret", result)
	}
	if len(events) != 2 || events[0] != "activate" || events[1] != "dialog" {
		t.Fatalf("events = %#v, want [activate dialog]", events)
	}
}

func TestReadDlgItemTextUsesDynamicBufferLength(t *testing.T) {
	originalGetDialogItemHandle := getDialogItemHandle
	originalGetWindowTextLength := getWindowTextLength
	originalGetDialogItemText := getDialogItemText
	originalGetComboSelection := getComboSelection
	defer func() {
		getDialogItemHandle = originalGetDialogItemHandle
		getWindowTextLength = originalGetWindowTextLength
		getDialogItemText = originalGetDialogItemText
		getComboSelection = originalGetComboSelection
	}()

	password := strings.Repeat("p", 600)
	getDialogItemHandle = func(hwnd, id uintptr) uintptr {
		return 1
	}
	getWindowTextLength = func(hwnd uintptr) int {
		return len(password)
	}
	getDialogItemText = func(hwnd, id uintptr, buf []uint16) int {
		copy(buf, utf16.Encode([]rune(password)))
		return len(password)
	}

	got := readDlgItemTextBytes(1, uintptr(idcPassword))
	if string(got) != password {
		t.Fatalf("readDlgItemText() len=%d, want %d", len(got), len(password))
	}
}

func TestShowWin32ModalRejectsMissingSelection(t *testing.T) {
	originalActivate := activateApprovalModalApplication
	originalShowDialog := showDialogBoxIndirectParam
	originalGetComboSelection := getComboSelection
	defer func() {
		activateApprovalModalApplication = originalActivate
		showDialogBoxIndirectParam = originalShowDialog
		getComboSelection = originalGetComboSelection
	}()

	activateApprovalModalApplication = func() {}
	getComboSelection = func(hwnd uintptr) int32 {
		return cbErr
	}
	showDialogBoxIndirectParam = func(template []byte) {
		dialogProc(1, uintptr(wmCommand), uintptr(idOK), 0)
	}

	result := showWin32Modal(approvalModalInput{
		Origin:         "http://127.0.0.1:4173",
		OrganizationID: "org-1",
		BusinessNo:     "2768603291",
		Action:         "ephemeral",
		Labels:         []string{"Test Certificate"},
		DefaultIndex:   0,
		Copy:           localization.ForLanguage(localization.English),
	})
	if result.Err == nil {
		t.Fatal("expected missing selection error")
	}
}
