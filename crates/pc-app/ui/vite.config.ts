import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// O front é servido pelo webview do Tauri, nunca por um servidor de verdade.
// `clearScreen: false` deixa os logs do cargo visíveis junto; a porta é fixa
// porque o `devUrl` do tauri.conf.json aponta pra ela.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    // Chromium/WebKit recentes — o webview embutido, não navegadores no mundo.
    target: "es2022",
    outDir: "dist",
    emptyOutDir: true,
  },
});
