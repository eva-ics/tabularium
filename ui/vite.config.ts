import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  build: {
    outDir: "dist",
    assetsDir: "assets",
    chunkSizeWarningLimit: 700,
  },
  server: {
    proxy: {
      "/api": { target: "http://127.0.0.1:3050", changeOrigin: true },
      "/rpc": { target: "http://127.0.0.1:3050", changeOrigin: true },
      "/ws": { target: "ws://127.0.0.1:3050", ws: true },
    },
  },
});
