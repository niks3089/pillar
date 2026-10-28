// Shared settings for the Playwright suites. Everything is overridable via env
// so CI and local runs can pick free ports / a different docker invocation.

const httpPort = Number(process.env.PILLAR_E2E_HTTP_PORT ?? 18080)
const grpcPort = Number(process.env.PILLAR_E2E_GRPC_PORT ?? 15051)

export const e2eEnv = {
  httpPort,
  grpcPort,
  baseURL: `http://127.0.0.1:${httpPort}`,
  adminUsername: 'admin',
  adminPassword: process.env.PILLAR_E2E_ADMIN_PASSWORD ?? 'pillar-e2e-password',
  authToken: process.env.PILLAR_E2E_AUTH_TOKEN ?? 'pillar-e2e-token',
  // System suite only: real agent + surfpool in a systemd container.
  systemEnabled: process.env.PILLAR_E2E_SYSTEM === '1',
  // `docker` in CI; `sudo docker` on dev boxes where the socket is root-only.
  docker: (process.env.PILLAR_E2E_DOCKER ?? 'docker').split(' '),
  surfpoolVersion: process.env.PILLAR_E2E_SURFPOOL_VERSION ?? '',
}
