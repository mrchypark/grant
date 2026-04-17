import { defineConfig } from "vite";
import { fileURLToPath, URL } from "node:url";

export default defineConfig({
  build: {
    rollupOptions: {
      input: {
        index: fileURLToPath(new URL("./index.html", import.meta.url)),
        script: fileURLToPath(new URL("./script.html", import.meta.url)),
      },
    },
  },
  root: fileURLToPath(new URL("./", import.meta.url)),
  server: {
    host: "127.0.0.1",
    port: 4173,
    proxy: {
      "/agent": {
        target: "http://127.0.0.1:8765",
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/agent/, ""),
      },
      "/gateway": {
        target: "http://127.0.0.1:18080",
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/gateway/, ""),
      },
      "/trailbase": {
        target: "http://127.0.0.1:4000",
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/trailbase/, ""),
      },
    },
  },
});
