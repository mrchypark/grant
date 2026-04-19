import type { BrowserCertificateIdentity, EscrowProofBundle } from "./client.ts";
import { base64ToBytes, bytesToBase64 } from "./base64.ts";
import type {
  Html5CertificateBundleInput,
  PreparedHtml5CertificateBundle,
} from "./html5.ts";
import {
  findHtml5CertificateFiles,
  prepareHtml5CertificateBundle,
  readHtml5CertificateBytes,
} from "./html5.ts";
import {
  inspectCertificateWithWasm,
  prepareChallengeProofWithWasm,
  signChallengeWithWasm,
} from "./npki-wasm.ts";

export type PreparedHtml5ChallengeProof = PreparedHtml5CertificateBundle & {
  challengeSignatureBase64: string;
};

export type PreparedHtml5SignedChallengeProof = {
  certificate: BrowserCertificateIdentity;
  challengeSignatureBase64: string;
  selection: PreparedHtml5CertificateBundle["selection"];
};

export async function prepareHtml5ChallengeProof(
  challengePayload: string,
  files: Iterable<File>,
  input: Html5CertificateBundleInput,
): Promise<PreparedHtml5ChallengeProof> {
  const selection = findHtml5CertificateFiles(files);
  const bytes = await readHtml5CertificateBytes(selection);
  const proof = await prepareChallengeProofWithWasm({
    ...bytes,
    businessNo: input.businessNo,
    commonName: input.commonName,
    password: input.password,
    challengePayload,
  });

  return {
    selection,
    bundle: { ...proof.bundle, password: input.password },
    challengeSignatureBase64: proof.challengeSignatureBase64,
  };
}

export async function prepareHtml5SignedChallengeProof(
  challengePayload: string,
  files: Iterable<File>,
  input: Html5CertificateBundleInput,
): Promise<PreparedHtml5SignedChallengeProof> {
  const selection = findHtml5CertificateFiles(files);
  const bytes = await readHtml5CertificateBytes(selection);
  const metadata = await inspectCertificateWithWasm(bytes.signCertDer);
  const challengeSignatureBase64 = await signChallengeWithWasm(
    bytes.signPriKey,
    input.password,
    challengePayload,
  );

  return {
    selection,
    certificate: {
      businessNo: metadata.businessNo || input.businessNo?.trim() || "",
      commonName: metadata.commonName || input.commonName?.trim() || "",
      signCertDerBase64: bytesToBase64(bytes.signCertDer),
    },
    challengeSignatureBase64,
  };
}

export async function signChallengeWithProofBundle(
  bundle: EscrowProofBundle,
  challengePayload: string,
): Promise<string> {
  const proof = await prepareChallengeProofWithWasm({
    signCertDer: base64ToBytes(bundle.signCertDerBase64),
    signPriKey: base64ToBytes(bundle.signPriKeyBase64),
    kmCertDer: base64ToBytes(bundle.kmCertDerBase64),
    kmPriKey: base64ToBytes(bundle.kmPriKeyBase64),
    businessNo: bundle.businessNo,
    commonName: bundle.commonName,
    password: bundle.password,
    challengePayload,
  });
  return proof.challengeSignatureBase64;
}

export async function signNpkiChallenge(
  signPriKeyDer: Uint8Array,
  password: string,
  challengePayload: string,
): Promise<string> {
  return signChallengeWithWasm(signPriKeyDer, password, challengePayload);
}
