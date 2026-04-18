import * as CertdWebSdkBrowser from "../../src/index.ts";

declare global {
  interface Window {
    WebSdkBrowser?: typeof CertdWebSdkBrowser;
    CertdWebSdkBrowser?: typeof CertdWebSdkBrowser;
  }
}

window.WebSdkBrowser = CertdWebSdkBrowser;
window.CertdWebSdkBrowser = CertdWebSdkBrowser;
