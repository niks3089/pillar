import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { e2eEnv } from '../env'

// Drives the systemd container that stands in for a validator host.

const ROOT = path.resolve(import.meta.dirname, '../../../..')
const IMAGE = 'pillar-e2e-node'

function docker(args: string[], opts: { input?: string; allowFail?: boolean } = {}): string {
  const [bin, ...pre] = e2eEnv.docker
  try {
    return execFileSync(bin, [...pre, ...args], {
      input: opts.input,
      encoding: 'utf8',
      stdio: ['pipe', 'pipe', 'pipe'],
      maxBuffer: 64 * 1024 * 1024,
    }).trim()
  } catch (err) {
    const e = err as { stdout?: string; stderr?: string; message: string }
    // Many probes (is-active, is-system-running) report via exit code but still
    // print the answer we want.
    if (opts.allowFail) return (e.stdout ?? '').trim()
    throw new Error(`docker ${args.join(' ')} failed: ${e.message}\n${e.stdout ?? ''}\n${e.stderr ?? ''}`)
  }
}

function hostUbuntuImage(): string {
  const osRelease = fs.readFileSync('/etc/os-release', 'utf8')
  const id = /^ID=(.*)$/m.exec(osRelease)?.[1]?.replace(/"/g, '')
  const version = /^VERSION_ID=(.*)$/m.exec(osRelease)?.[1]?.replace(/"/g, '')
  return id === 'ubuntu' && version ? `ubuntu:${version}` : 'ubuntu:24.04'
}

export class TestNode {
  readonly container: string

  constructor(readonly nodeId: string) {
    this.container = `pillar-e2e-${nodeId}`
  }

  /** Build the agent from this checkout, boot the container, run the real installer. */
  start(): void {
    // PILLAR_E2E_AGENT_BINARY runs the suite against another agent build, e.g.
    // an older release, to confirm a test really catches a regression.
    const agentBinary = process.env.PILLAR_E2E_AGENT_BINARY ?? path.join(ROOT, 'target/debug/agent')
    if (!process.env.PILLAR_E2E_AGENT_BINARY && process.env.PILLAR_E2E_SKIP_BUILD !== '1') {
      execFileSync('cargo', ['build', '-q', '-p', 'pillar-agent'], { cwd: ROOT, stdio: 'inherit' })
    }
    docker([
      'build', '-q', '-t', IMAGE,
      '--build-arg', `BASE_IMAGE=${hostUbuntuImage()}`,
      path.join(import.meta.dirname, '../node'),
    ])
    this.remove()
    docker([
      'run', '-d', '--name', this.container, '--hostname', this.nodeId,
      '--privileged', '--cgroupns=host', '-v', '/sys/fs/cgroup:/sys/fs/cgroup:rw',
      '--add-host=host.docker.internal:host-gateway',
      '-v', `${agentBinary}:/opt/pillar-agent:ro`,
      '-v', `${path.join(ROOT, 'scripts/install-node.sh')}:/opt/install-node.sh:ro`,
      IMAGE,
    ])
    this.waitForSystemd()
    this.exec([
      'bash', '/opt/install-node.sh',
      '--binary', '/opt/pillar-agent',
      '--skip-toolchains',
      '--cluster', 'devnet',
      '--controller', `http://host.docker.internal:${e2eEnv.grpcPort}`,
      '--token', e2eEnv.authToken,
      '--node-id', this.nodeId,
    ])
  }

  private waitForSystemd(): void {
    for (let i = 0; i < 60; i++) {
      const state = docker(['exec', this.container, 'systemctl', 'is-system-running'], { allowFail: true })
      if (state === 'running' || state === 'degraded') return
      execFileSync('sleep', ['1'])
    }
    throw new Error('systemd did not come up in the test node container')
  }

  exec(cmd: string[], opts: { allowFail?: boolean } = {}): string {
    return docker(['exec', this.container, ...cmd], opts)
  }

  /** Run a command as the `sol` user, the same identity pillar scripts run as. */
  execAsSol(script: string, opts: { allowFail?: boolean } = {}): string {
    return this.exec(['sudo', '-u', 'sol', '-H', 'bash', '-lc', script], opts)
  }

  unitState(unit: string): string {
    // is-active exits non-zero for inactive units; we want the word either way.
    return this.exec(['systemctl', 'is-active', unit], { allowFail: true }) || 'inactive'
  }

  fileExists(file: string): boolean {
    return this.exec(['bash', '-c', `test -e '${file}' && echo yes`], { allowFail: true }) === 'yes'
  }

  journal(unit: string, lines = 200): string {
    return this.exec(['journalctl', '-u', unit, '-n', String(lines), '--no-pager'], { allowFail: true })
  }

  remove(): void {
    docker(['rm', '-f', this.container], { allowFail: true })
  }
}
