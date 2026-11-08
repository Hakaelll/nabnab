import { defineConfig } from "vite";
export default defineConfig({
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**", "**/.cache/**", "**/artifacts/**"] } },
  build: { target: "es2022", rollupOptions: { input: ["index.html", "quick-skip.html"] } },
});


