import {
  WebSdk,
  configureHtml5CertificateInput,
  findHtml5CertificateFiles,
} from "./index.ts";
import { configureNpkiWasmAssets } from "./npki-wasm.ts";

declare global {
  interface Window {
    WebSdkBrowser?: {
      WebSdk: typeof WebSdk;
      CertdWebSdk: typeof WebSdk;
      configureHtml5CertificateInput: typeof configureHtml5CertificateInput;
      findHtml5CertificateFiles: typeof findHtml5CertificateFiles;
    };
    CertdWebSdkBrowser?: {
      WebSdk: typeof WebSdk;
      CertdWebSdk: typeof WebSdk;
      configureHtml5CertificateInput: typeof configureHtml5CertificateInput;
      findHtml5CertificateFiles: typeof findHtml5CertificateFiles;
    };
  }
}

if (typeof window !== "undefined") {
  const currentScript = document.currentScript;
  if (currentScript instanceof HTMLScriptElement && currentScript.src) {
    const baseUrl = new URL("./", currentScript.src);
    configureNpkiWasmAssets({
      wasmUrl: new URL("./npki_signer.wasm", baseUrl),
      wasmExecUrl: new URL("./wasm_exec.js", baseUrl),
    });
  }
  const sdk = {
    WebSdk,
    CertdWebSdk: WebSdk,
    configureHtml5CertificateInput,
    findHtml5CertificateFiles,
  };
  window.WebSdkBrowser = sdk;
  window.CertdWebSdkBrowser = sdk;
}

export * from "./index.ts";
