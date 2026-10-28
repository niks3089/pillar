import { expect, test, type Page } from '@playwright/test'
import { e2eEnv } from '../env'
import { TestNode } from '../support/node'

// End-to-end regression for "I stopped it in Pillar but it kept turning back
// on": a real controller, real install-node.sh, real pillar-agent under
// systemd, real surfpool. Agent timings are the shipped defaults (20s health
// checks, 3 consecutive misses before acting), so the "stays stopped" window
// below is long enough that the old agent reliably restarted the validator.

const NODE_ID = `e2e-${Date.now().toString(36)}`
const UNIT = 'surfpool'
const HOLD = `/home/sol/.pillar/${UNIT}.hold`
// 3 checks × 20s + check latency + margin. The pre-fix agent restarted within ~70s.
const STAYS_DOWN_MS = 150_000
const RECOVERS_WITHIN_MS = 180_000

const node = new TestNode(NODE_ID)

async function lifecycleState(page: Page): Promise<string> {
  const res = await page.request.get(`/api/nodes/${NODE_ID}`)
  if (!res.ok()) return `http-${res.status()}`
  return (await res.json()).lifecycle_state
}

async function expectState(page: Page, state: string, timeout: number) {
  await expect.poll(() => lifecycleState(page), { timeout, intervals: [2_000, 5_000] }).toBe(state)
}

/** Assert the unit stays down (and the UI says stopped) for the whole window. */
async function expectStaysStopped(page: Page, windowMs: number, midway?: () => void) {
  const deadline = Date.now() + windowMs
  let ranMidway = false
  while (Date.now() < deadline) {
    expect(node.unitState(UNIT), 'validator was restarted while held').not.toMatch(/^(active|activating)$/)
    expect(await lifecycleState(page)).toBe('stopped')
    if (midway && !ranMidway && Date.now() > deadline - windowMs / 2) {
      midway()
      ranMidway = true
    }
    await page.waitForTimeout(10_000)
  }
}

async function confirm(page: Page, title: string, button: string) {
  const dialog = page.getByRole('dialog', { name: title })
  await expect(dialog).toBeVisible()
  await dialog.getByRole('button', { name: button, exact: true }).click()
}

interface Execution {
  description?: string
  exit_code?: number
  completed_at?: number
  script?: string
  stdout?: string
  stderr?: string
}

async function latestExecution(page: Page, description: RegExp): Promise<Execution | undefined> {
  const res = await page.request.get(`/api/nodes/${NODE_ID}/scripts`)
  const rows: Execution[] = await res.json()
  return rows.find(r => description.test(r.description ?? ''))
}

async function surfpoolVersion(page: Page): Promise<string> {
  if (e2eEnv.surfpoolVersion) return e2eEnv.surfpoolVersion
  const res = await page.request.get('/api/client-releases/surfpool')
  const versions: string[] = (await res.json()).versions ?? []
  expect(versions.length, 'no surfpool releases available').toBeGreaterThan(0)
  return versions[0]
}

test.describe.serial('validator stop/start lifecycle on a real node', () => {
  test.setTimeout(15 * 60_000)

  test.beforeAll(() => node.start())

  test.afterAll(() => {
    if (process.env.PILLAR_E2E_KEEP_NODE !== '1') node.remove()
  })

  test.afterEach(async ({}, testInfo) => {
    if (testInfo.status !== testInfo.expectedStatus) {
      await testInfo.attach('pillar-agent journal', { body: node.journal('pillar-agent', 400) })
      await testInfo.attach(`${UNIT} journal`, { body: node.journal(UNIT) })
    }
  })

  test('installed agent registers with the controller', async ({ page }) => {
    await expect.poll(() => lifecycleState(page), { timeout: 60_000 }).not.toMatch(/^http-/)
    await page.goto('/')
    await expect(page.getByText(NODE_ID).first()).toBeVisible()
  })

  test('surfpool provisioned through the UI comes up healthy', async ({ page }) => {
    const version = await surfpoolVersion(page)
    await page.goto(`/nodes/${NODE_ID}`)
    await page.getByRole('button', { name: 'Setup Validator Now' }).click()
    await page.getByLabel('Client').selectOption('surfpool')
    await page.getByLabel('Cluster').selectOption('devnet')
    await page.getByLabel('Version').fill(version)
    await page.getByRole('button', { name: 'Install Validator' }).click()
    await confirm(page, 'Install Validator', 'Install')

    await expectState(page, 'healthy', 8 * 60_000)
    expect(node.unitState(UNIT)).toBe('active')

    const provision = await latestExecution(page, /^Provision surfpool/)
    expect(provision?.exit_code).toBe(0)
    expect(provision?.script).toContain(`sudo systemctl enable --now ${UNIT}`)
    expect(provision?.stderr).toContain(`+pillar$ sudo systemctl enable --now ${UNIT}`)
    await page.reload()
    await expect(page.getByTestId('lifecycle-state')).toHaveText('healthy')
  })

  test('Stop keeps the validator down, even across an agent restart', async ({ page }) => {
    await page.goto(`/nodes/${NODE_ID}`)
    await page.getByRole('button', { name: 'Stop', exact: true }).click()

    // What the operator is shown before confirming...
    const dialog = page.getByRole('dialog', { name: 'Stop Validator' })
    await dialog.getByRole('button', { name: 'Show commands' }).click()
    const preview = await dialog.getByLabel('Commands to run').innerText()
    expect(preview).toContain(`sudo systemctl stop ${UNIT}`)
    await confirm(page, 'Stop Validator', 'Stop')

    // ...is exactly what was recorded, and every command shows up as it ran.
    await expect.poll(async () => (await latestExecution(page, /^Stop validator$/))?.completed_at, { timeout: 60_000 })
      .toBeTruthy()
    const stop = (await latestExecution(page, /^Stop validator$/))!
    expect(stop.exit_code).toBe(0)
    expect(stop.script?.trim()).toBe(preview.trim())
    expect(stop.stderr).toContain(`+pillar$ sudo systemctl stop ${UNIT}`)
    expect(node.journal('pillar-agent', 400)).toContain(`[cmd] sudo systemctl stop ${UNIT}`)

    await expectState(page, 'stopped', 90_000)
    expect(node.fileExists(HOLD)).toBe(true)
    expect(node.unitState(UNIT)).not.toBe('active')

    // Halfway through, bounce the agent: a fresh agent process must still honour the hold.
    await expectStaysStopped(page, STAYS_DOWN_MS, () => node.exec(['systemctl', 'restart', 'pillar-agent']))

    await page.reload()
    await expect(page.getByTestId('lifecycle-state')).toHaveText('stopped')
    await expect(page.getByRole('button', { name: 'Start', exact: true })).toBeVisible()
    const activity = page.getByRole('region', { name: 'Activity' })
    await expect(activity.getByRole('listitem').filter({ hasText: 'Stop validator' }).first()).toContainText('succeeded')
  })

  test('Start clears the hold and the validator returns to healthy', async ({ page }) => {
    await page.goto(`/nodes/${NODE_ID}`)
    await page.getByRole('button', { name: 'Start', exact: true }).click()
    await confirm(page, 'Start Validator', 'Start')

    await expectState(page, 'healthy', RECOVERS_WITHIN_MS)
    expect(node.fileExists(HOLD)).toBe(false)
    expect(node.unitState(UNIT)).toBe('active')
    await page.reload()
    await expect(page.getByRole('button', { name: 'Restart', exact: true })).toBeVisible()
  })

  test('an unplanned stop (no hold) is still auto-recovered by the agent', async ({ page }) => {
    // Guard the other direction: the hold must not have disabled outage recovery.
    node.execAsSol(`sudo systemctl stop ${UNIT}`)
    expect(node.unitState(UNIT)).not.toBe('active')
    await expect.poll(() => node.unitState(UNIT), { timeout: RECOVERS_WITHIN_MS, intervals: [5_000] }).toBe('active')
    await expectState(page, 'healthy', RECOVERS_WITHIN_MS)
  })

  test('an operator hold created from the shell is honoured', async ({ page }) => {
    // The documented manual workflow: touch the hold file, then systemctl stop.
    node.execAsSol(`mkdir -p /home/sol/.pillar && touch ${HOLD} && sudo systemctl stop ${UNIT}`)
    await expectState(page, 'stopped', 90_000)
    await expectStaysStopped(page, STAYS_DOWN_MS)

    node.execAsSol(`rm ${HOLD} && sudo systemctl start ${UNIT}`)
    await expectState(page, 'healthy', RECOVERS_WITHIN_MS)
  })
})
