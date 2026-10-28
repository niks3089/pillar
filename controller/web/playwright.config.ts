import path from 'node:path'
import { defineConfig, devices } from '@playwright/test'
import { e2eEnv } from './playwright/env'
import { authStatePath } from './playwright/support/auth'

// Two layers, same harness:
//   ui     — real controller + browser; node API responses are stubbed where a
//            test needs a node in a specific state. Fast, runs on every PR.
//   system — real controller + a systemd container running install-node.sh,
//            pillar-agent and surfpool. Opt in with PILLAR_E2E_SYSTEM=1.
export default defineConfig({
  testDir: './playwright/tests',
  fullyParallel: false,
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  workers: 1,
  timeout: 60_000,
  reporter: process.env.CI ? [['github'], ['html', { open: 'never' }]] : [['list']],
  use: {
    baseURL: e2eEnv.baseURL,
    trace: 'retain-on-failure',
    ...devices['Desktop Chrome'],
  },
  webServer: {
    command: 'bash playwright/support/start-controller.sh',
    cwd: import.meta.dirname,
    url: `${e2eEnv.baseURL}/api/auth/check`,
    reuseExistingServer: false,
    stdout: 'pipe',
    stderr: 'pipe',
    // First run compiles the controller.
    timeout: 600_000,
  },
  projects: [
    { name: 'public', testMatch: /login\.e2e\.spec\.ts/ },
    { name: 'auth-setup', testMatch: /auth\.global\.setup\.ts/ },
    {
      name: 'ui',
      testMatch: /.*\.e2e\.spec\.ts/,
      testIgnore: /login\.e2e\.spec\.ts/,
      dependencies: ['auth-setup'],
      use: { storageState: authStatePath },
    },
    ...(e2eEnv.systemEnabled
      ? [{
          name: 'system',
          testMatch: /.*\.system\.spec\.ts/,
          dependencies: ['auth-setup'],
          use: { storageState: authStatePath },
        }]
      : []),
  ],
  outputDir: path.join(import.meta.dirname, 'test-results'),
})
