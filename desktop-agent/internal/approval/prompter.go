//go:build darwin || windows

package approval

import (
	"context"
	"fmt"
	"strings"

	"github.com/cypark/account/desktop-agent/internal/httpapi"
	"github.com/cypark/account/desktop-agent/internal/localization"
)

type Prompter struct{}

type approvalModalInput struct {
	Origin         string
	OrganizationID string
	BusinessNo     string
	Action         string
	Labels         []string
	DefaultIndex   int
	Copy           localization.Strings
}

var approvalModalSlot = make(chan struct{}, 1)

var acquireApprovalModal = func(ctx context.Context) (func(), error) {
	select {
	case approvalModalSlot <- struct{}{}:
		return func() {
			<-approvalModalSlot
		}, nil
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}

// 플랫폼 모달 hook. 각 플랫폼 파일 init()에서 네이티브 호출로 바인딩한다.
var (
	resetApprovalModalCancellation   func()
	requestApprovalModalCancellation func()
	activateApprovalModalApplication func()
	runApprovalModal                 func(approvalModalInput) (string, bool)
)

// 기본 showApprovalModal은 runApprovalModal 단일 호출. 플랫폼 init()은 필요 시
// 추가 준비/복원 단계(macOS activation policy 등)를 감싸 덮어쓸 수 있다.
var showApprovalModal = func(input approvalModalInput) (string, bool) {
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
	releaseModal, err := acquireApprovalModal(ctx)
	if err != nil {
		return httpapi.ApprovalDecision{}, err
	}
	defer releaseModal()

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
		copy := localization.ForLanguage(localization.SystemLanguage())
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
			Copy:           copy,
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
