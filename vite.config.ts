/// <reference types="vitest" />

import { fileURLToPath, URL } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],
  resolve: {
    alias: [
      {
        find: "@localflow/sdk/react",
        replacement: fileURLToPath(
          new URL("./packages/localflow-sdk/src/react.ts", import.meta.url),
        ),
      },
      {
        find: "@localflow/sdk/adapters/tauri",
        replacement: fileURLToPath(
          new URL("./packages/localflow-sdk/src/adapters/tauri.ts", import.meta.url),
        ),
      },
      {
        find: "@localflow/sdk",
        replacement: fileURLToPath(
          new URL("./packages/localflow-sdk/src/index.ts", import.meta.url),
        ),
      },
    ],
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
  test: {
    environment: "jsdom",
    globals: true,
    include: ["tests/**/*.test.ts", "tests/**/*.test.tsx"],
  },
}));
