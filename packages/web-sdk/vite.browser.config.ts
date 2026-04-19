import { defineConfig } from "vite";
import { fileURLToPath, URL } from "node:url";

export default defineConfig({
  build: {
    emptyOutDir: true,
    lib: {
      entry: fileURLToPath(new URL("./src/browser-global.ts", import.meta.url)),
      fileName: () => "certd-web-sdk.global.js",
      formats: ["iife"],
      name: "WebSdkBrowser",
    },
    outDir: fileURLToPath(new URL("./dist/browser", import.meta.url)),
  },
});
