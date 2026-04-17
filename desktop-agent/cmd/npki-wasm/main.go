//go:build js && wasm

package main

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"syscall/js"

	"github.com/cypark/account/desktop-agent/internal/npki"
)

type prepareBundleRequest struct {
	SignCertDerBase64 string `json:"signCertDerBase64"`
	SignPriKeyBase64  string `json:"signPriKeyBase64"`
	KMCertDerBase64   string `json:"kmCertDerBase64"`
	KMPriKeyBase64    string `json:"kmPriKeyBase64"`
	BusinessNo        string `json:"businessNo"`
	CommonName        string `json:"commonName"`
}

type prepareChallengeProofRequest struct {
	prepareBundleRequest
	PasswordBase64   string `json:"passwordBase64"`
	ChallengePayload string `json:"challengePayload"`
}

type inspectCertificateRequest struct {
	SignCertDerBase64 string `json:"signCertDerBase64"`
}

type inspectCertificateResponse struct {
	CommonName string `json:"commonName"`
	BusinessNo string `json:"businessNo"`
}

type signChallengeOnlyRequest struct {
	SignPriKeyBase64 string `json:"signPriKeyBase64"`
	PasswordBase64   string `json:"passwordBase64"`
	ChallengePayload string `json:"challengePayload"`
}

type signChallengeOnlyResponse struct {
	ChallengeSignatureBase64 string `json:"challengeSignatureBase64"`
}

func main() {
	api := js.Global().Get("Object").New()
	api.Set("inspectCertificate", wrap(handleInspectCertificate))
	api.Set("prepareBundle", wrap(handlePrepareBundle))
	api.Set("prepareChallengeProof", wrap(handlePrepareChallengeProof))
	api.Set("signChallengeOnly", wrap(handleSignChallengeOnly))
	js.Global().Set("CertdWebSdkNpkiWasm", api)

	select {}
}

func wrap(fn func(string) (string, error)) js.Func {
	return js.FuncOf(func(this js.Value, args []js.Value) any {
		if len(args) != 1 || args[0].Type() != js.TypeString {
			return responseObject("", fmt.Errorf("expected exactly one JSON string argument"))
		}
		return responseObject(fn(args[0].String()))
	})
}

func responseObject(result string, err error) js.Value {
	obj := js.Global().Get("Object").New()
	if err != nil {
		obj.Set("ok", false)
		obj.Set("error", err.Error())
		return obj
	}
	obj.Set("ok", true)
	obj.Set("result", result)
	return obj
}

func handleInspectCertificate(raw string) (string, error) {
	var request inspectCertificateRequest
	if err := json.Unmarshal([]byte(raw), &request); err != nil {
		return "", fmt.Errorf("parse inspectCertificate request: %w", err)
	}
	signCertDER, err := decodeBase64(request.SignCertDerBase64)
	if err != nil {
		return "", fmt.Errorf("decode signCert.der: %w", err)
	}

	metadata, err := npki.InspectCertificate(signCertDER)
	if err != nil {
		return "", err
	}

	return marshal(inspectCertificateResponse{
		CommonName: metadata.CommonName,
		BusinessNo: metadata.BusinessNo,
	})
}

func handlePrepareBundle(raw string) (string, error) {
	var request prepareBundleRequest
	if err := json.Unmarshal([]byte(raw), &request); err != nil {
		return "", fmt.Errorf("parse prepareBundle request: %w", err)
	}

	input, err := decodeBundleInput(request)
	if err != nil {
		return "", err
	}

	bundle, err := npki.PrepareBrowserBundle(input)
	if err != nil {
		return "", err
	}

	return marshal(bundle)
}

func handlePrepareChallengeProof(raw string) (string, error) {
	var request prepareChallengeProofRequest
	if err := json.Unmarshal([]byte(raw), &request); err != nil {
		return "", fmt.Errorf("parse prepareChallengeProof request: %w", err)
	}

	input, err := decodeBundleInput(request.prepareBundleRequest)
	if err != nil {
		return "", err
	}
	password, err := decodePasswordBase64(request.PasswordBase64)
	if err != nil {
		return "", fmt.Errorf("decode password: %w", err)
	}
	defer zeroBytes(password)

	proof, err := npki.PrepareBrowserChallengeProof(npki.BrowserChallengeProofInput{
		BrowserBundleInput: input,
		Password:           password,
		Challenge:          request.ChallengePayload,
	})
	if err != nil {
		return "", err
	}

	return marshal(proof)
}

func handleSignChallengeOnly(raw string) (string, error) {
	var request signChallengeOnlyRequest
	if err := json.Unmarshal([]byte(raw), &request); err != nil {
		return "", fmt.Errorf("parse signChallengeOnly request: %w", err)
	}

	signPriKey, err := decodeBase64(request.SignPriKeyBase64)
	if err != nil {
		return "", fmt.Errorf("decode signPri.key: %w", err)
	}
	password, err := decodePasswordBase64(request.PasswordBase64)
	if err != nil {
		return "", fmt.Errorf("decode password: %w", err)
	}
	defer zeroBytes(password)

	signature, err := npki.SignChallengeForBrowser(signPriKey, password, request.ChallengePayload)
	if err != nil {
		return "", err
	}

	return marshal(signChallengeOnlyResponse{
		ChallengeSignatureBase64: signature,
	})
}

func decodeBundleInput(request prepareBundleRequest) (npki.BrowserBundleInput, error) {
	signCertDER, err := decodeBase64(request.SignCertDerBase64)
	if err != nil {
		return npki.BrowserBundleInput{}, fmt.Errorf("decode signCert.der: %w", err)
	}
	signPriKey, err := decodeBase64(request.SignPriKeyBase64)
	if err != nil {
		return npki.BrowserBundleInput{}, fmt.Errorf("decode signPri.key: %w", err)
	}
	kmCertDER, err := decodeBase64(request.KMCertDerBase64)
	if err != nil {
		return npki.BrowserBundleInput{}, fmt.Errorf("decode kmCert.der: %w", err)
	}
	kmPriKey, err := decodeBase64(request.KMPriKeyBase64)
	if err != nil {
		return npki.BrowserBundleInput{}, fmt.Errorf("decode kmPri.key: %w", err)
	}

	return npki.BrowserBundleInput{
		SignCertDER: signCertDER,
		SignPriKey:  signPriKey,
		KMCertDER:   kmCertDER,
		KMPriKey:    kmPriKey,
		BusinessNo:  request.BusinessNo,
		CommonName:  request.CommonName,
	}, nil
}

func decodeBase64(value string) ([]byte, error) {
	return base64.StdEncoding.DecodeString(value)
}

func decodePasswordBase64(value string) ([]byte, error) {
	return base64.StdEncoding.DecodeString(value)
}

func marshal(value any) (string, error) {
	bytes, err := json.Marshal(value)
	if err != nil {
		return "", err
	}
	return string(bytes), nil
}

func zeroBytes(buf []byte) {
	clear(buf)
}
