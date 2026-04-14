package localization

import "testing"

func TestLanguageFromTag(t *testing.T) {
	testCases := []struct {
		name string
		tag  string
		want Language
	}{
		{name: "korean locale uses korean", tag: "ko-KR", want: Korean},
		{name: "short korean locale uses korean", tag: "ko", want: Korean},
		{name: "english locale uses english", tag: "en-US", want: English},
		{name: "japanese locale uses english", tag: "ja-JP", want: English},
		{name: "empty locale falls back to korean", tag: "", want: Korean},
	}

	for _, tc := range testCases {
		t.Run(tc.name, func(t *testing.T) {
			if got := languageFromTag(tc.tag); got != tc.want {
				t.Fatalf("languageFromTag(%q) = %q, want %q", tc.tag, got, tc.want)
			}
		})
	}
}

func TestStringsForLanguage(t *testing.T) {
	ko := ForLanguage(Korean)
	if ko.AppName != "REPass" {
		t.Fatalf("korean app name = %q, want REPass", ko.AppName)
	}
	if ko.OpenExamplePage != "예제 페이지 열기" {
		t.Fatalf("korean open label = %q", ko.OpenExamplePage)
	}

	en := ForLanguage(English)
	if en.OpenExamplePage != "Open Example Page" {
		t.Fatalf("english open label = %q", en.OpenExamplePage)
	}
	if en.ApprovalTitle != "REPass Approval Request" {
		t.Fatalf("english approval title = %q", en.ApprovalTitle)
	}
	if ko.LocalCertificateSource != "로컬" {
		t.Fatalf("korean local source label = %q", ko.LocalCertificateSource)
	}
	if ko.RemovableCertificateSource != "이동식디스크" {
		t.Fatalf("korean removable source label = %q", ko.RemovableCertificateSource)
	}
	if en.PKCS12CertificateSource != "PKCS#12" {
		t.Fatalf("english pkcs12 source label = %q", en.PKCS12CertificateSource)
	}
}
