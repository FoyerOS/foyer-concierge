import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// In dev, API calls are proxied to a locally running daemon
// (`concierge daemon --config dev.toml`). In production the daemon serves
// the built assets itself (cargo feature `webgui`).
export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      "/api": "http://127.0.0.1:8080",
    },
  },
});
