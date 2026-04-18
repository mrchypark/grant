//go:build js && wasm

package main

import (
	"encoding/json"
	"fmt"
	"syscall/js"

	"github.com/cypark/account/desktop-agent/internal/npki"
)

type inspectCertificateResponse struct {
	CommonName string `json:"commonName"`
	BusinessNo string `json:"businessNo"`
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
	js.Global().Set(resolveExportName(), api)

	select {}
}

func resolveExportName() string {
	name := js.Global().Get("__CERTD_WEB_SDK_NPKI_WASM_EXPORT_NAME__")
	if name.Type() == js.TypeString && name.String() != "" {
		return name.String()
	}
	return "CertdWebSdkNpkiWasm"
}

func wrap(fn func([]js.Value) (string, error)) js.Func {
	return js.FuncOf(func(this js.Value, args []js.Value) any {
		return responseObject(fn(args))
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

func handleInspectCertificate(args []js.Value) (string, error) {
	signCertDER, err := byteArg(args, 0, "signCert.der")
	if err != nil {
		return "", err
	}
	defer zeroBytes(signCertDER)

	metadata, err := npki.InspectCertificate(signCertDER)
	if err != nil {
		return "", err
	}

	return marshal(inspectCertificateResponse{
		CommonName: metadata.CommonName,
		BusinessNo: metadata.BusinessNo,
	})
}

func handlePrepareBundle(args []js.Value) (string, error) {
	input, err := decodeBundleInput(args)
	if err != nil {
		return "", err
	}
	defer zeroBrowserBundleInput(input)

	bundle, err := npki.PrepareBrowserBundle(input)
	if err != nil {
		return "", err
	}

	return marshal(bundle)
}

func handlePrepareChallengeProof(args []js.Value) (string, error) {
	input, err := decodeBundleInput(args)
	if err != nil {
		return "", err
	}
	defer zeroBrowserBundleInput(input)
	password, err := byteArg(args, 6, "password")
	if err != nil {
		return "", err
	}
	defer zeroBytes(password)
	challengePayload, err := stringArg(args, 7, "challengePayload")
	if err != nil {
		return "", err
	}

	proof, err := npki.PrepareBrowserChallengeProof(npki.BrowserChallengeProofInput{
		BrowserBundleInput: input,
		Password:           password,
		Challenge:          challengePayload,
	})
	if err != nil {
		return "", err
	}

	return marshal(proof)
}

func handleSignChallengeOnly(args []js.Value) (string, error) {
	signPriKey, err := byteArg(args, 0, "signPri.key")
	if err != nil {
		return "", err
	}
	defer zeroBytes(signPriKey)
	password, err := byteArg(args, 1, "password")
	if err != nil {
		return "", err
	}
	defer zeroBytes(password)
	challengePayload, err := stringArg(args, 2, "challengePayload")
	if err != nil {
		return "", err
	}

	signature, err := npki.SignChallengeForBrowser(signPriKey, password, challengePayload)
	if err != nil {
		return "", err
	}

	return marshal(signChallengeOnlyResponse{
		ChallengeSignatureBase64: signature,
	})
}

func decodeBundleInput(args []js.Value) (npki.BrowserBundleInput, error) {
	signCertDER, err := byteArg(args, 0, "signCert.der")
	if err != nil {
		return npki.BrowserBundleInput{}, err
	}
	signPriKey, err := byteArg(args, 1, "signPri.key")
	if err != nil {
		zeroBytes(signCertDER)
		return npki.BrowserBundleInput{}, err
	}
	kmCertDER, err := byteArg(args, 2, "kmCert.der")
	if err != nil {
		zeroBytes(signCertDER)
		zeroBytes(signPriKey)
		return npki.BrowserBundleInput{}, err
	}
	kmPriKey, err := byteArg(args, 3, "kmPri.key")
	if err != nil {
		zeroBytes(signCertDER)
		zeroBytes(signPriKey)
		zeroBytes(kmCertDER)
		return npki.BrowserBundleInput{}, err
	}
	businessNo, err := stringArg(args, 4, "businessNo")
	if err != nil {
		zeroBytes(signCertDER)
		zeroBytes(signPriKey)
		zeroBytes(kmCertDER)
		zeroBytes(kmPriKey)
		return npki.BrowserBundleInput{}, err
	}
	commonName, err := stringArg(args, 5, "commonName")
	if err != nil {
		zeroBytes(signCertDER)
		zeroBytes(signPriKey)
		zeroBytes(kmCertDER)
		zeroBytes(kmPriKey)
		return npki.BrowserBundleInput{}, err
	}

	return npki.BrowserBundleInput{
		SignCertDER: signCertDER,
		SignPriKey:  signPriKey,
		KMCertDER:   kmCertDER,
		KMPriKey:    kmPriKey,
		BusinessNo:  businessNo,
		CommonName:  commonName,
	}, nil
}

func byteArg(args []js.Value, index int, label string) ([]byte, error) {
	if len(args) <= index {
		return nil, fmt.Errorf("missing %s argument", label)
	}
	value := args[index]
	if value.Type() != js.TypeObject {
		return nil, fmt.Errorf("%s must be Uint8Array", label)
	}
	byteLength := value.Get("byteLength")
	if byteLength.Type() != js.TypeNumber {
		return nil, fmt.Errorf("%s must be Uint8Array", label)
	}
	buf := make([]byte, byteLength.Int())
	if copied := js.CopyBytesToGo(buf, value); copied != len(buf) {
		zeroBytes(buf)
		return nil, fmt.Errorf("copy %s bytes: expected %d, got %d", label, len(buf), copied)
	}
	return buf, nil
}

func stringArg(args []js.Value, index int, label string) (string, error) {
	if len(args) <= index {
		return "", fmt.Errorf("missing %s argument", label)
	}
	if args[index].Type() != js.TypeString {
		return "", fmt.Errorf("%s must be string", label)
	}
	return args[index].String(), nil
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

func zeroBrowserBundleInput(input npki.BrowserBundleInput) {
	zeroBytes(input.SignCertDER)
	zeroBytes(input.SignPriKey)
	zeroBytes(input.KMCertDER)
	zeroBytes(input.KMPriKey)
}
