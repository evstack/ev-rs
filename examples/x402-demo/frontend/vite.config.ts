import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    host: true, // Allow external connections (for Docker)
    proxy: {
      "/api": process.env.VITE_API_URL || "http://localhost:3000",
      "/auth": process.env.VITE_API_URL || "http://localhost:3000",
      "/wallet": process.env.VITE_API_URL || "http://localhost:3000",
      "/health": process.env.VITE_API_URL || "http://localhost:3000",
      "/rpc": {
        target: process.env.VITE_RPC_URL || "http://127.0.0.1:8545",
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/rpc/, ""),
      },
    },
  },
});
