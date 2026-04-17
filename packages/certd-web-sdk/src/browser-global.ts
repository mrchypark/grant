import * as CertdWebSdkBrowser from "./index.ts";

declare global {
  interface Window {
    CertdWebSdkBrowser?: typeof CertdWebSdkBrowser;
    __CERTD_WEB_SDK_WASM_BASE_URL__?: string;
  }
}

if (typeof window !== "undefined") {
  const currentScript = document.currentScript;
  if (currentScript instanceof HTMLScriptElement && currentScript.src) {
    window.__CERTD_WEB_SDK_WASM_BASE_URL__ = new URL("./", currentScript.src).toString();
  }
  window.CertdWebSdkBrowser = CertdWebSdkBrowser;
}

export * from "./index.ts";
