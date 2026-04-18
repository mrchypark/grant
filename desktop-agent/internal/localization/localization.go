package localization

import "strings"

type Language string

const (
	Korean  Language = "ko"
	English Language = "en"
)

type Strings struct {
	AppName             string
	AppTooltip          string
	StatusListening     string
	OpenExamplePage     string
	OpenExampleTooltip  string
	QuitApp             string
	QuitAppTooltip      string
	ApprovalTitle       string
	ApprovalInfoFormat  string
	Approve             string
	Cancel              string
	CertificateLabel    string
	PasswordLabel       string
	PasswordPlaceholder string
	LocalCertificateSource     string
	RemovableCertificateSource string
	NPKICertificateSource      string
	GPKICertificateSource      string
	PKCS12CertificateSource    string
	BundleCertificateSource    string
	UnknownCertificateName     string
}

func languageFromTag(tag string) Language {
	normalized := strings.TrimSpace(strings.ToLower(tag))
	if normalized == "" {
		return Korean
	}
	normalized = strings.ReplaceAll(normalized, "_", "-")
	primary := normalized
	if idx := strings.Index(primary, "-"); idx >= 0 {
		primary = primary[:idx]
	}
	if primary == "ko" {
		return Korean
	}
	return English
}

func ForLanguage(language Language) Strings {
	switch language {
	case English:
		return Strings{
			AppName:             "REPass",
			AppTooltip:          "REPass desktop agent",
			StatusListening:     "REPass listening on %s",
			OpenExamplePage:     "Open Example Page",
			OpenExampleTooltip:  "open the local browser example",
			QuitApp:             "Quit REPass",
			QuitAppTooltip:      "quit REPass",
			ApprovalTitle:       "REPass Approval Request",
			ApprovalInfoFormat:  "Origin: %s\nOrganization: %s\nBusiness Number: %s\nAction: %s\n\nChoose the certificate to use and enter its password.",
			Approve:             "Approve",
			Cancel:              "Cancel",
			CertificateLabel:    "Certificate",
			PasswordLabel:       "Password",
			PasswordPlaceholder: "Certificate password",
			LocalCertificateSource:     "Local",
			RemovableCertificateSource: "Removable",
			NPKICertificateSource:      "NPKI",
			GPKICertificateSource:      "GPKI",
			PKCS12CertificateSource:    "PKCS#12",
			BundleCertificateSource:    "Bundle",
			UnknownCertificateName:     "(Unnamed certificate)",
		}
	default:
		return Strings{
			AppName:             "REPass",
			AppTooltip:          "REPass 데스크톱 에이전트",
			StatusListening:     "REPass 실행 중: %s",
			OpenExamplePage:     "예제 페이지 열기",
			OpenExampleTooltip:  "로컬 브라우저 예제를 엽니다",
			QuitApp:             "REPass 종료",
			QuitAppTooltip:      "REPass를 종료합니다",
			ApprovalTitle:       "REPass 승인 요청",
			ApprovalInfoFormat:  "Origin: %s\n조직: %s\n사업자번호: %s\n동작: %s\n\n사용할 인증서를 선택하고 비밀번호를 입력하세요.",
			Approve:             "승인",
			Cancel:              "취소",
			CertificateLabel:    "공동인증서",
			PasswordLabel:       "비밀번호",
			PasswordPlaceholder: "공동인증서 비밀번호",
			LocalCertificateSource:     "로컬",
			RemovableCertificateSource: "이동식디스크",
			NPKICertificateSource:      "NPKI",
			GPKICertificateSource:      "GPKI",
			PKCS12CertificateSource:    "PKCS#12",
			BundleCertificateSource:    "인증서 번들",
			UnknownCertificateName:     "(이름 없음)",
		}
	}
}
