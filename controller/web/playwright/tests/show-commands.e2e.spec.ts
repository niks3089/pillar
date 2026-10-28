import { expect, test } from '@playwright/test'
import { serveNode, stubNode } from '../support/stub-node'

// Node GETs are stubbed; previews (`?dry_run=1`) and history hit the real
// controller, so these assert on scripts rendered by the real templates.

test.describe('show commands before running an action', () => {
  test('Stop dialog shows the exact script and previewing sends nothing', async ({ page }) => {
    const { actions } = await serveNode(page, stubNode({ node_id: 'cmd-stop' }))
    await page.goto('/nodes/cmd-stop')

    await page.getByRole('button', { name: 'Stop', exact: true }).click()
    const dialog = page.getByRole('dialog', { name: 'Stop Validator' })
    await dialog.getByRole('button', { name: 'Show commands' }).click()

    const script = dialog.getByLabel('Commands to run')
    // No provision config → the controller's default unit.
    await expect(script).toContainText('/home/sol/.pillar/solana-validator.hold')
    await expect(script).toContainText('sudo systemctl stop solana-validator')
    const text = await script.innerText()
    expect(text.indexOf('.hold')).toBeLessThan(text.indexOf('systemctl stop'))

    await dialog.getByRole('button', { name: 'Hide commands' }).click()
    await expect(script).toHaveCount(0)
    await dialog.getByRole('button', { name: 'Cancel' }).click()

    expect(actions).toEqual([])
    const history = await page.request.get('/api/nodes/cmd-stop/scripts')
    expect(await history.json()).toEqual([])
  })

  test('Start dialog shows the hold being cleared before the restart', async ({ page }) => {
    const node = stubNode({ node_id: 'cmd-start', lifecycle_state: 'stopped' })
    await serveNode(page, node)
    await page.goto('/nodes/cmd-start')

    await page.getByRole('button', { name: 'Start', exact: true }).click()
    const dialog = page.getByRole('dialog', { name: 'Start Validator' })
    await dialog.getByRole('button', { name: 'Show commands' }).click()
    const script = dialog.getByLabel('Commands to run')
    await expect(script).toContainText('rm -f /home/sol/.pillar/solana-validator.hold')
    await expect(script).toContainText('sudo systemctl restart solana-validator')
  })

  test('provision preview renders the full script with secrets masked', async ({ page }) => {
    const { actions } = await serveNode(page, stubNode({ node_id: 'cmd-provision' }))
    await page.goto('/nodes/cmd-provision')

    await page.getByRole('button', { name: 'Edit Config' }).click()
    await page.getByLabel('Client').selectOption('surfpool')
    await page.getByLabel('Cluster').selectOption('devnet')
    await page.getByLabel('Version').fill('1.7.0')
    await page.getByRole('button', { name: 'Show Advanced' }).click()
    await page.getByLabel('Environment Variables').fill('HELIUS_API_KEY=abc123secret\nRUST_LOG=info')
    await page.getByRole('button', { name: 'Update Validator' }).last().click()

    const dialog = page.getByRole('dialog', { name: 'Update Validator' })
    await dialog.getByRole('button', { name: 'Show commands' }).click()
    const script = dialog.getByLabel('Commands to run')
    await expect(script).toContainText('surfpool-linux-x64.tar.gz')
    await expect(script).toContainText('sudo systemctl enable --now surfpool')
    await expect(script).toContainText('Environment=HELIUS_API_KEY=***')
    await expect(script).toContainText('Environment=RUST_LOG=info')
    await expect(script).not.toContainText('abc123secret')

    await dialog.getByRole('button', { name: 'Cancel' }).click()
    expect(actions).toEqual([])
  })
})

test('Activity panel lists past actions with script, commands and output', async ({ page }) => {
  await serveNode(page, stubNode({ node_id: 'cmd-activity' }))
  const now = Math.floor(Date.now() / 1000)
  await page.route('**/api/nodes/cmd-activity/scripts?*', route => route.fulfill({
    json: [
      {
        script_id: 's2', description: 'Stop validator', initiated_at: now - 60, completed_at: now - 58,
        exit_code: 0, timed_out: false,
        script: 'sudo systemctl stop surfpool',
        stderr: '+pillar$ sudo systemctl stop surfpool\n', stdout: 'Stopped surfpool\n',
      },
      {
        script_id: 's1', description: 'Restart validator', initiated_at: now - 600, completed_at: now - 590,
        exit_code: 1, timed_out: false, error: 'Unit surfpool.service not found.',
        script: 'sudo systemctl restart surfpool', stderr: 'Unit surfpool.service not found.\n',
      },
    ],
  }))
  await page.goto('/nodes/cmd-activity')

  const activity = page.getByRole('region', { name: 'Activity' })
  const stop = activity.getByRole('listitem').filter({ hasText: 'Stop validator' })
  await expect(stop).toContainText('succeeded')
  await stop.getByText('Stop validator').click()
  await expect(stop.getByLabel('Script')).toHaveText('sudo systemctl stop surfpool')
  await expect(stop.getByLabel('Commands & errors (stderr)')).toContainText('+pillar$ sudo systemctl stop surfpool')
  await expect(stop.getByLabel('Output (stdout)')).toContainText('Stopped surfpool')

  const restart = activity.getByRole('listitem').filter({ hasText: 'Restart validator' })
  await expect(restart).toContainText('failed (exit 1)')
  await restart.getByText('Restart validator').click()
  await expect(restart).toContainText('Unit surfpool.service not found.')
})
