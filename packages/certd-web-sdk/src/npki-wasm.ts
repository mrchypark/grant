import { bytesToBase64 } from "./base64.ts";
import type { ProofBundle } from "./client.ts";

const DEFAULT_WASM_FILE = "npki_signer.wasm";
const DEFAULT_WASM_EXEC_FILE = "wasm_exec.js";
const READY_POLL_INTERVAL_MS = 10;
const READY_TIMEOUT_MS = 5_000;

type WasmEnvelope = {
  ok?: boolean;
  result?: string;
  error?: string;
};

type NpkiWasmApi = {
  inspectCertificate(inputJson: string): WasmEnvelope;
  prepareBundle(inputJson: string): WasmEnvelope;
  prepareChallengeProof(inputJson: string): WasmEnvelope;
  signChallengeOnly(inputJson: string): WasmEnvelope;
};

type GoRuntime = {
  importObject: WebAssembly.Imports;
  run(instance: WebAssembly.Instance): Promise<void>;
};

type NpkiWasmAssetUrls = {
  wasmUrl?: string | URL;
  wasmExecUrl?: string | URL;
};

export type WasmCertificateMetadata = {
  commonName: string;
  businessNo: string;
};

export type WasmChallengeProof = {
  bundle: ProofBundle;
  challengeSignatureBase64: string;
};

declare global {
  interface Window {
    CertdWebSdkNpkiWasm?: NpkiWasmApi;
    __CERTD_WEB_SDK_WASM_BASE_URL__?: string;
  }

  var Go: undefined | {
    new (): GoRuntime;
  };

  var CertdWebSdkNpkiWasm: undefined | NpkiWasmApi;
}

let assetUrls: NpkiWasmAssetUrls = {};
let runtimeReady: Promise<void> | null = null;
let apiReady: Promise<NpkiWasmApi> | null = null;
let wasmRunPromise: Promise<void> | null = null;

export function configureNpkiWasmAssets(urls: NpkiWasmAssetUrls): void {
  assetUrls = {
    wasmUrl: urls.wasmUrl ?? assetUrls.wasmUrl,
    wasmExecUrl: urls.wasmExecUrl ?? assetUrls.wasmExecUrl,
  };
}

export async function inspectCertificateWithWasm(
  signCertDer: Uint8Array,
): Promise<WasmCertificateMetadata> {
  return callNpkiWasm<WasmCertificateMetadata>("inspectCertificate", {
    signCertDerBase64: bytesToBase64(signCertDer),
  });
}

export async function prepareBundleWithWasm(input: {
  signCertDer: Uint8Array;
  signPriKey: Uint8Array;
  kmCertDer: Uint8Array;
  kmPriKey: Uint8Array;
  businessNo?: string;
  commonName?: string;
}): Promise<ProofBundle> {
  return callNpkiWasm<ProofBundle>("prepareBundle", encodeBundleInput(input));
}

export async function prepareChallengeProofWithWasm(input: {
  signCertDer: Uint8Array;
  signPriKey: Uint8Array;
  kmCertDer: Uint8Array;
  kmPriKey: Uint8Array;
  businessNo?: string;
  commonName?: string;
  password: string;
  challengePayload: string;
}): Promise<WasmChallengeProof> {
  return callNpkiWasm<WasmChallengeProof>("prepareChallengeProof", {
    ...encodeBundleInput(input),
    password: input.password,
    challengePayload: input.challengePayload,
  });
}

export async function signChallengeWithWasm(
  signPriKey: Uint8Array,
  password: string,
  challengePayload: string,
): Promise<string> {
  const result = await callNpkiWasm<{ challengeSignatureBase64: string }>("signChallengeOnly", {
    signPriKeyBase64: bytesToBase64(signPriKey),
    password,
    challengePayload,
  });
  return result.challengeSignatureBase64;
}

async function callNpkiWasm<T>(method: keyof NpkiWasmApi, payload: object): Promise<T> {
  const api = await ensureNpkiWasmApi();
  const response = api[method](JSON.stringify(payload));
  if (!response?.ok || typeof response.result !== "string") {
    throw new Error(response?.error ?? "failed to invoke NPKI wasm runtime");
  }
  return JSON.parse(response.result) as T;
}

async function ensureNpkiWasmApi(): Promise<NpkiWasmApi> {
  if (globalThis.CertdWebSdkNpkiWasm) {
    return globalThis.CertdWebSdkNpkiWasm;
  }
  if (apiReady) {
    return apiReady;
  }

  apiReady = (async () => {
    await ensureGoRuntime();
    const wasmBinary = await readBinary(resolveAssetUrl("wasmUrl", DEFAULT_WASM_FILE));
    const go = new globalThis.Go!();
    const result = await WebAssembly.instantiate(wasmBinary, go.importObject);
    wasmRunPromise = go.run(result.instance);
    await waitForApi();
    return globalThis.CertdWebSdkNpkiWasm!;
  })();

  return apiReady;
}

async function ensureGoRuntime(): Promise<void> {
  if (globalThis.Go) {
    return;
  }
  if (runtimeReady) {
    return runtimeReady;
  }

  runtimeReady = (async () => {
    const runtimeSource = await readText(resolveAssetUrl("wasmExecUrl", DEFAULT_WASM_EXEC_FILE));
    globalThis.eval(`${runtimeSource}\n//# sourceURL=certd-web-sdk-wasm_exec.js`);
    if (!globalThis.Go) {
      throw new Error("failed to initialize Go wasm runtime");
    }
  })();

  return runtimeReady;
}

async function waitForApi(): Promise<void> {
  const startedAt = Date.now();
  while (!globalThis.CertdWebSdkNpkiWasm) {
    if (Date.now() - startedAt > READY_TIMEOUT_MS) {
      throw new Error("timed out waiting for NPKI wasm runtime");
    }
    await new Promise((resolve) => setTimeout(resolve, READY_POLL_INTERVAL_MS));
  }
}

function encodeBundleInput(input: {
  signCertDer: Uint8Array;
  signPriKey: Uint8Array;
  kmCertDer: Uint8Array;
  kmPriKey: Uint8Array;
  businessNo?: string;
  commonName?: string;
}): Record<string, string> {
  return {
    signCertDerBase64: bytesToBase64(input.signCertDer),
    signPriKeyBase64: bytesToBase64(input.signPriKey),
    kmCertDerBase64: bytesToBase64(input.kmCertDer),
    kmPriKeyBase64: bytesToBase64(input.kmPriKey),
    businessNo: input.businessNo?.trim() ?? "",
    commonName: input.commonName?.trim() ?? "",
  };
}

function resolveAssetUrl(key: keyof NpkiWasmAssetUrls, fileName: string): URL {
  const configured = assetUrls[key];
  if (configured) {
    if (configured instanceof URL) {
      return configured;
    }
    return new URL(configured, globalThis.location?.href ?? "file:///");
  }

  if (typeof window !== "undefined" && window.__CERTD_WEB_SDK_WASM_BASE_URL__) {
    return new URL(fileName, ensureTrailingSlash(window.__CERTD_WEB_SDK_WASM_BASE_URL__));
  }

  return new URL(`./generated/${fileName}`, import.meta.url);
}

async function readBinary(url: URL): Promise<ArrayBuffer> {
  if (url.protocol === "file:") {
    const { readFile } = await loadNodeFsPromises();
    const buffer = await readFile(url);
    return buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
  }

  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`failed to load ${url.toString()}: ${response.status}`);
  }
  return response.arrayBuffer();
}

async function readText(url: URL): Promise<string> {
  if (url.protocol === "file:") {
    const { readFile } = await loadNodeFsPromises();
    return readFile(url, "utf8");
  }

  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`failed to load ${url.toString()}: ${response.status}`);
  }
  return response.text();
}

function ensureTrailingSlash(value: string): string {
  return value.endsWith("/") ? value : `${value}/`;
}

async function loadNodeFsPromises() {
  const dynamicImport = new Function("specifier", "return import(specifier);") as (
    specifier: string,
  ) => Promise<typeof import("node:fs/promises")>;
  return dynamicImport("node:fs/promises");
}
