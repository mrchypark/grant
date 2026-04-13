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

func TestPromptApprovalActivatesAppBeforeShowingModal(t *testing.T) {
	originalShow := showApprovalModal
	originalRun := runApprovalModal
	originalActivate := activateApprovalModalApplication
	defer func() {
		showApprovalModal = originalShow
		runApprovalModal = originalRun
		activateApprovalModalApplication = originalActivate
	}()

	events := make([]string, 0, 2)
	activateApprovalModalApplication = func() {
		events = append(events, "activate")
	}
	runApprovalModal = func(input approvalModalInput) (string, bool) {
		events = append(events, "show")
		return "0\x1fsecret-password", true
	}

	decision, err := NewPrompter().PromptApproval(context.Background(), httpapi.ApprovalPrompt{
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
	if decision.CertificateID != "cert-1" {
		t.Fatalf("certificateID = %q, want cert-1", decision.CertificateID)
	}
	if decision.Password != "secret-password" {
		t.Fatalf("password = %q, want secret-password", decision.Password)
	}
	if len(events) != 2 || events[0] != "activate" || events[1] != "show" {
		t.Fatalf("events = %#v, want [activate show]", events)
	}
}
