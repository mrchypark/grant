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

type approvalModalOutput struct {
	SelectedIndex int
	Password      string
	Approved      bool
	Err           error
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
	runApprovalModal                 func(approvalModalInput) approvalModalOutput
)

// 기본 showApprovalModal은 runApprovalModal 단일 호출. 플랫폼 init()은 필요 시
// 추가 준비/복원 단계(macOS activation policy 등)를 감싸 덮어쓸 수 있다.
var showApprovalModal = func(input approvalModalInput) approvalModalOutput {
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
	resultCh := make(chan approvalModalOutput, 1)
	go func() {
		copy := localization.ForLanguage(localization.SystemLanguage())
		businessNo := prompt.BusinessNo
		if strings.TrimSpace(businessNo) == "" {
			businessNo = "-"
		}
		result := showApprovalModal(approvalModalInput{
			Origin:         prompt.Origin,
			OrganizationID: prompt.OrganizationID,
			BusinessNo:     businessNo,
			Action:         prompt.Action,
			Labels:         joinedLabels,
			DefaultIndex:   defaultIndex,
			Copy:           copy,
		})
		resultCh <- result
	}()

	var result approvalModalOutput
	select {
	case <-ctx.Done():
		requestApprovalModalCancellation()
		result = <-resultCh
		if result.Err != nil {
			return httpapi.ApprovalDecision{}, result.Err
		}
		return httpapi.ApprovalDecision{}, ctx.Err()
	case result = <-resultCh:
	}
	if result.Err != nil {
		return httpapi.ApprovalDecision{}, result.Err
	}
	if !result.Approved {
		return httpapi.ApprovalDecision{}, httpapi.ErrApprovalRejected
	}
	if result.SelectedIndex < 0 || result.SelectedIndex >= len(prompt.Certificates) {
		return httpapi.ApprovalDecision{}, fmt.Errorf("selected certificate index %d out of range", result.SelectedIndex)
	}
	if result.Password == "" {
		return httpapi.ApprovalDecision{}, httpapi.ErrApprovalRejected
	}

	return httpapi.ApprovalDecision{
		CertificateID: prompt.Certificates[result.SelectedIndex].ID,
		Password:      result.Password,
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
