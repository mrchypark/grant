import type { ProofBundle } from "./client.ts";
import { base64ToBytes } from "./base64.ts";
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
  prepareChallengeProofWithWasm,
  signChallengeWithWasm,
} from "./npki-wasm.ts";

export type PreparedHtml5ChallengeProof = PreparedHtml5CertificateBundle & {
  challengeSignatureBase64: string;
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
    password: input.password.trim(),
    challengePayload,
  });

  return {
    selection,
    bundle: { ...proof.bundle, password: input.password.trim() },
    challengeSignatureBase64: proof.challengeSignatureBase64,
  };
}

export async function signChallengeWithProofBundle(
  bundle: ProofBundle,
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
