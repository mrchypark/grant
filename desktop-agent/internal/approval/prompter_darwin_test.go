//go:build darwin

package approval

import (
	"context"
	"errors"
	"reflect"
	"strconv"
	"sync"
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
	originalPrepare := prepareApprovalModalApplication
	defer func() {
		showApprovalModal = originalShow
		runApprovalModal = originalRun
		prepareApprovalModalApplication = originalPrepare
	}()

	events := make([]string, 0, 3)
	prepareApprovalModalApplication = func() func() {
		events = append(events, "prepare")
		return func() {
			events = append(events, "restore")
		}
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
	if len(events) != 3 || events[0] != "prepare" || events[1] != "show" || events[2] != "restore" {
		t.Fatalf("events = %#v, want [prepare show restore]", events)
	}
}

func TestPrepareApprovalModalApplicationRestoresOriginalPolicyOnce(t *testing.T) {
	originalPrepareApplicationPolicy := prepareApprovalApplicationPolicy
	originalRestoreApplicationPolicy := restoreApprovalApplicationPolicy
	originalActivate := activateApprovalModalApplication
	originalDepth := approvalAppPolicyDepth
	originalPolicy := approvalAppPreviousPolicy
	defer func() {
		prepareApprovalApplicationPolicy = originalPrepareApplicationPolicy
		restoreApprovalApplicationPolicy = originalRestoreApplicationPolicy
		activateApprovalModalApplication = originalActivate
		approvalAppPolicyDepth = originalDepth
		approvalAppPreviousPolicy = originalPolicy
	}()

	approvalAppPolicyDepth = 0
	approvalAppPreviousPolicy = 0

	var mu sync.Mutex
	events := make([]string, 0, 4)
	prepareApprovalApplicationPolicy = func() int64 {
		mu.Lock()
		events = append(events, "prepare")
		mu.Unlock()
		return 7
	}
	restoreApprovalApplicationPolicy = func(policy int64) {
		mu.Lock()
		events = append(events, "restore:"+fmtInt64(policy))
		mu.Unlock()
	}
	activateApprovalModalApplication = func() {
		mu.Lock()
		events = append(events, "activate")
		mu.Unlock()
	}

	restoreOuter := prepareApprovalModalApplication()
	restoreInner := prepareApprovalModalApplication()
	restoreOuter()
	restoreInner()

	want := []string{"prepare", "activate", "restore:7"}
	if !reflect.DeepEqual(events, want) {
		t.Fatalf("events = %#v, want %#v", events, want)
	}
}

func fmtInt64(value int64) string {
	return strconv.FormatInt(value, 10)
}
