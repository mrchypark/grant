//go:build darwin

package approval

import (
	"context"
	"errors"
	"reflect"
	"strconv"
	"sync"
	"sync/atomic"
	"testing"
	"time"

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
	runApprovalModal = func(input approvalModalInput) approvalModalOutput {
		events = append(events, "show")
		return approvalModalOutput{SelectedIndex: 0, Password: []byte("secret-password"), Approved: true}
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
	if string(decision.Password) != "secret-password" {
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

func TestPromptApprovalSerializesConcurrentModals(t *testing.T) {
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
	requestApprovalModalCancellation = func() {}

	firstStarted := make(chan struct{})
	firstRelease := make(chan struct{})
	secondStarted := make(chan struct{}, 1)
	var calls atomic.Int32

	showApprovalModal = func(input approvalModalInput) approvalModalOutput {
		switch calls.Add(1) {
		case 1:
			close(firstStarted)
			<-firstRelease
			return approvalModalOutput{SelectedIndex: 0, Password: []byte("first-password"), Approved: true}
		case 2:
			secondStarted <- struct{}{}
			return approvalModalOutput{SelectedIndex: 0, Password: []byte("second-password"), Approved: true}
		default:
			t.Fatalf("showApprovalModal called more than twice")
			return approvalModalOutput{}
		}
	}

	firstDone := make(chan error, 1)
	go func() {
		_, err := NewPrompter().PromptApproval(context.Background(), approvalPromptFixture("cert-1"))
		firstDone <- err
	}()

	<-firstStarted

	secondDone := make(chan error, 1)
	go func() {
		_, err := NewPrompter().PromptApproval(context.Background(), approvalPromptFixture("cert-2"))
		secondDone <- err
	}()

	select {
	case <-secondStarted:
		t.Fatal("second modal started before first modal finished")
	case <-time.After(150 * time.Millisecond):
	}

	close(firstRelease)

	select {
	case <-secondStarted:
	case <-time.After(2 * time.Second):
		t.Fatal("second modal did not start after first modal finished")
	}

	select {
	case err := <-firstDone:
		if err != nil {
			t.Fatalf("first PromptApproval returned err: %v", err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("first PromptApproval did not finish")
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

func TestPromptApprovalCancelWhileWaitingDoesNotCancelActiveModal(t *testing.T) {
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
	requestApprovalModalCancellation = func() {
		select {
		case cancelRequested <- struct{}{}:
		default:
		}
	}

	firstStarted := make(chan struct{})
	firstRelease := make(chan struct{})
	var calls atomic.Int32
	showApprovalModal = func(input approvalModalInput) approvalModalOutput {
		switch calls.Add(1) {
		case 1:
			close(firstStarted)
			<-firstRelease
			return approvalModalOutput{SelectedIndex: 0, Password: []byte("secret-password"), Approved: true}
		case 2:
			t.Fatal("waiting prompt reached showApprovalModal before the active modal completed")
			return approvalModalOutput{}
		default:
			t.Fatalf("showApprovalModal called more than twice")
			return approvalModalOutput{}
		}
	}

	firstDone := make(chan error, 1)
	go func() {
		_, err := NewPrompter().PromptApproval(context.Background(), approvalPromptFixture("cert-1"))
		firstDone <- err
	}()

	<-firstStarted

	ctx, cancel := context.WithCancel(context.Background())
	secondDone := make(chan error, 1)
	go func() {
		_, err := NewPrompter().PromptApproval(ctx, approvalPromptFixture("cert-2"))
		secondDone <- err
	}()

	cancel()

	select {
	case <-cancelRequested:
		t.Fatal("waiting prompt canceled the active modal")
	case <-time.After(150 * time.Millisecond):
	}

	select {
	case err := <-secondDone:
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("second PromptApproval err = %v, want context.Canceled", err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("second PromptApproval did not finish after cancellation")
	}

	close(firstRelease)

	select {
	case err := <-firstDone:
		if err != nil {
			t.Fatalf("first PromptApproval returned err: %v", err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("first PromptApproval did not finish")
	}
}

func TestCertificateOptionLabelIncludesSourceMetadata(t *testing.T) {
	copy := localization.ForLanguage(localization.Korean)

	localLabel := certificateOptionLabel(npki.CertificateMetadata{
		ID:          "bundle:/Users/test/NPKI/yessign/USER/cn=test",
		CommonName:  "로컬 인증서",
		SourceKind:  npki.CertificateSourceKindBundle,
		StorageKind: npki.CertificateStorageKindNPKI,
		DisplayPath: "NPKI/yessign/USER/cn=test",
	}, copy)
	if localLabel != "로컬 인증서 | 로컬 NPKI | NPKI/yessign/USER/cn=test" {
		t.Fatalf("local label = %q", localLabel)
	}

	removableLabel := certificateOptionLabel(npki.CertificateMetadata{
		ID:          "pkcs12:/Volumes/Lexar/protected.p12",
		CommonName:  "이동식 인증서",
		SourceKind:  npki.CertificateSourceKindPKCS12,
		StorageKind: npki.CertificateStorageKindPKCS12,
		DisplayPath: "protected.p12",
		Removable:   true,
	}, copy)
	if removableLabel != "이동식 인증서 | 이동식디스크 PKCS#12 | protected.p12" {
		t.Fatalf("removable label = %q", removableLabel)
	}

	unnamedLabel := certificateOptionLabel(npki.CertificateMetadata{
		ID:          "pkcs12:/Users/test/unnamed.p12",
		SourceKind:  npki.CertificateSourceKindPKCS12,
		StorageKind: npki.CertificateStorageKindPKCS12,
		DisplayPath: "unnamed.p12",
	}, copy)
	if unnamedLabel != "unnamed.p12 | 로컬 PKCS#12" {
		t.Fatalf("unnamed label = %q", unnamedLabel)
	}
}

func approvalPromptFixture(certificateID string) httpapi.ApprovalPrompt {
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

func fmtInt64(value int64) string {
	return strconv.FormatInt(value, 10)
}
