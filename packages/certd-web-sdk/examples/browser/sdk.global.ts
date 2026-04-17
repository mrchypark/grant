import * as CertdWebSdkBrowser from "../../src/index.ts";

declare global {
  interface Window {
    CertdWebSdkBrowser?: typeof CertdWebSdkBrowser;
  }
}

window.CertdWebSdkBrowser = CertdWebSdkBrowser;
