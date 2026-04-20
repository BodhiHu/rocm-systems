import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  timeout: 30_000,
  retries: 0,
  workers: 1,
  fullyParallel: false,
  use: {
    baseURL: "http://localhost:5173",
    headless: true,
  },
  webServer: [
    {
      command: "npx tsx server/index.ts",
      port: 50051,
      reuseExistingServer: !process.env.CI,
      env: { PORT: "50051" },
    },
    {
      command: "npx vite --port 5173",
      port: 5173,
      reuseExistingServer: !process.env.CI,
    },
  ],
  projects: [
    {
      name: "chromium",
      use: { browserName: "chromium" },
    },
  ],
});
