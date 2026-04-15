//go:build darwin || windows

package approval

import (
	"context"
	"errors"
	"sync/atomic"
	"testing"
	"time"

	"github.com/cypark/account/desktop-agent/internal/httpapi"
	"github.com/cypark/account/desktop-agent/internal/npki"
)

func TestPromptApprovalWaitsForCanceledModalToExitBeforeReleasingSlot(t *testing.T) {
	originalShow := showApprovalModal
	originalRun := runApprovalModal
	originalReset := resetApprovalModalCancellation
	originalCancel := requestApprovalModalCancellation
	defer func() {
		showApprovalModal = originalShow
		runApprovalModal = originalRun
		resetApprovalModalCancellation = originalReset
		requestApprovalModalCancellation = originalCancel
	}()

	resetApprovalModalCancellation = func() {}
	cancelRequested := make(chan struct{}, 1)
	modalExit := make(chan struct{})
	firstStarted := make(chan struct{})
	secondStarted := make(chan struct{}, 1)
	var calls atomic.Int32

	showApprovalModal = func(input approvalModalInput) approvalModalOutput {
		switch calls.Add(1) {
		case 1:
			close(firstStarted)
			<-modalExit
			return approvalModalOutput{}
		case 2:
			secondStarted <- struct{}{}
			return approvalModalOutput{SelectedIndex: 0, Password: "secret", Approved: true}
		default:
			t.Fatalf("showApprovalModal called more than twice")
			return approvalModalOutput{}
		}
	}
	requestApprovalModalCancellation = func() {
		select {
		case cancelRequested <- struct{}{}:
		default:
		}
	}

	firstCtx, firstCancel := context.WithCancel(context.Background())
	firstDone := make(chan error, 1)
	go func() {
		_, err := NewPrompter().PromptApproval(firstCtx, approvalPromptFixtureForTest("cert-1"))
		firstDone <- err
	}()

	<-firstStarted
	firstCancel()

	select {
	case <-cancelRequested:
	case <-time.After(2 * time.Second):
		t.Fatal("expected cancellation request to be forwarded to modal")
	}

	secondDone := make(chan error, 1)
	go func() {
		_, err := NewPrompter().PromptApproval(context.Background(), approvalPromptFixtureForTest("cert-2"))
		secondDone <- err
	}()

	select {
	case <-secondStarted:
		t.Fatal("second modal started before canceled modal exited")
	case <-time.After(150 * time.Millisecond):
	}

	select {
	case err := <-firstDone:
		t.Fatalf("first PromptApproval returned early with %v", err)
	case <-time.After(150 * time.Millisecond):
	}

	close(modalExit)

	select {
	case err := <-firstDone:
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("first PromptApproval err = %v, want context.Canceled", err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("first PromptApproval did not finish after modal exit")
	}

	select {
	case <-secondStarted:
	case <-time.After(2 * time.Second):
		t.Fatal("second modal did not start after first modal exited")
	}

	select {
	case err := <-secondDone:
		if err != nil {
			t.Fatalf("second PromptApproval returned err: %v", err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("second PromptApproval did not finish")
	}
}

func approvalPromptFixtureForTest(certificateID string) httpapi.ApprovalPrompt {
	return httpapi.ApprovalPrompt{
		Origin:         "http://127.0.0.1:4173",
		Action:         "ephemeral",
		OrganizationID: "org-1",
		BusinessNo:     "2768603291",
		Certificates: []npki.CertificateMetadata{{
			ID:         certificateID,
			CommonName: "Test Certificate",
			BusinessNo: "2768603291",
		}},
	}
}
