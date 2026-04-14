//go:build windows

package approval

import (
	"context"
	"errors"
	"testing"
	"time"

	"github.com/cypark/account/desktop-agent/internal/httpapi"
	"github.com/cypark/account/desktop-agent/internal/localization"
	"github.com/cypark/account/desktop-agent/internal/npki"
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

	showApprovalModal = func(input approvalModalInput) (string, bool) {
		close(started)
		<-release
		return "", false
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
	runApprovalModal = func(input approvalModalInput) (string, bool) {
		received = input
		return "0\x1fsecret", true
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
