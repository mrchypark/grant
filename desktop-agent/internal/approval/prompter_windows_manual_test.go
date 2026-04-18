//go:build windows && manual

package approval

import (
	"testing"

	"desktop-agent/internal/localization"
)

// go test -tags manual -count=1 ./internal/approval -run TestManualDialog -v
// 실제 Win32 모달을 띄워 레이아웃/로컬라이제이션/입력 동작을 눈으로 확인한다.
// CI에서는 돌지 않는다.
func TestManualDialog(t *testing.T) {
	copy := localization.ForLanguage(localization.SystemLanguage())
	result := runApprovalModal(approvalModalInput{
		Origin:         "http://127.0.0.1:4173",
		OrganizationID: "org-test",
		BusinessNo:     "1234567890",
		Action:         "ephemeral",
		Labels:         []string{"테스트 인증서 A", "테스트 인증서 B"},
		DefaultIndex:   0,
		Copy:           copy,
	})
	t.Logf("selected=%d approved=%v password_len=%d err=%v", result.SelectedIndex, result.Approved, len(result.Password), result.Err)
}
