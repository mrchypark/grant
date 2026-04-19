package npki

import (
	"encoding/base64"
	"strings"
)

type BrowserBundleInput struct {
	SignCertDER []byte
	SignPriKey  []byte
	KMCertDER   []byte
	KMPriKey    []byte
	BusinessNo  string
	CommonName  string
}

type BrowserChallengeProofInput struct {
	BrowserBundleInput
	Password  []byte
	Challenge string
}

type BrowserChallengeProof struct {
	Bundle                   ProofBundle `json:"bundle"`
	ChallengeSignatureBase64 string      `json:"challengeSignatureBase64"`
}

func InspectCertificate(signCertDER []byte) (CertificateMetadata, error) {
	return metadataFromCertificate(bundleEntry{}, signCertDER)
}

func PrepareBrowserBundle(input BrowserBundleInput) (ProofBundle, error) {
	metadata, err := InspectCertificate(input.SignCertDER)
	if err != nil {
		return ProofBundle{}, err
	}
	businessNo := metadata.BusinessNo
	if businessNo == "" {
		businessNo = digitsOnly(input.BusinessNo)
	}
	commonName := metadata.CommonName
	if commonName == "" {
		commonName = strings.TrimSpace(input.CommonName)
	}

	return ProofBundle{
		BusinessNo:        businessNo,
		CommonName:        commonName,
		SignCertDerBase64: base64.StdEncoding.EncodeToString(input.SignCertDER),
		SignPriKeyBase64:  base64.StdEncoding.EncodeToString(input.SignPriKey),
		KMCertDerBase64:   base64.StdEncoding.EncodeToString(input.KMCertDER),
		KMPriKeyBase64:    base64.StdEncoding.EncodeToString(input.KMPriKey),
	}, nil
}

func PrepareBrowserChallengeProof(input BrowserChallengeProofInput) (BrowserChallengeProof, error) {
	password := append([]byte(nil), input.Password...)
	defer zeroBytes(password)

	bundle, err := PrepareBrowserBundle(input.BrowserBundleInput)
	if err != nil {
		return BrowserChallengeProof{}, err
	}

	challengeSignatureBase64, err := signChallenge(input.SignPriKey, password, input.Challenge)
	if err != nil {
		return BrowserChallengeProof{}, err
	}

	return BrowserChallengeProof{
		Bundle:                   bundle,
		ChallengeSignatureBase64: challengeSignatureBase64,
	}, nil
}

func SignChallengeForBrowser(signPriKey []byte, password []byte, challenge string) (string, error) {
	clonedPassword := append([]byte(nil), password...)
	defer zeroBytes(clonedPassword)

	return signChallenge(signPriKey, clonedPassword, challenge)
}
