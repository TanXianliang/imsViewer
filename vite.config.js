import { defineConfig } from "vite";

export default defineConfig({
  // Tauri 开发服务器配置
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
});
