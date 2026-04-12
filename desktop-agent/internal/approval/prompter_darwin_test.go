//go:build darwin

package approval

import (
	"context"
	"errors"
	"testing"
	"time"

	"github.com/cypark/account/desktop-agent/internal/httpapi"
	"github.com/cypark/account/desktop-agent/internal/npki"
)

func TestPromptApprovalCancelsModalWhenContextIsDone(t *testing.T) {
	originalShow := showApprovalModal
	originalReset := resetApprovalModalCancellation
	originalCancel := requestApprovalModalCancellation
	defer func() {
		showApprovalModal = originalShow
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
