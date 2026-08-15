import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  // Bazel points this at a materialized directory of specs (Playwright's
  // scanner does not follow runfiles symlinks); locally it is ./tests.
  testDir: process.env.PLAYWRIGHT_TEST_DIR || './tests',
  testMatch: '**/*.spec.ts',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 0,
  reporter: 'list',
  use: {
    trace: 'on-first-retry',
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
  // No webServer: each worker boots its own wview on an ephemeral port via
  // the fixture in tests/wview.ts, so runs never collide on a port.
});
