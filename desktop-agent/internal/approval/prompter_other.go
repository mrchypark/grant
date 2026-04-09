//go:build !darwin

package approval

import (
	"context"
	"fmt"

	"github.com/cypark/account/desktop-agent/internal/httpapi"
)

type Prompter struct{}

func NewPrompter() *Prompter {
	return &Prompter{}
}

func (p *Prompter) PromptApproval(context.Context, httpapi.ApprovalPrompt) (httpapi.ApprovalDecision, error) {
	return httpapi.ApprovalDecision{}, fmt.Errorf("native approval modal is not implemented on this platform")
}
