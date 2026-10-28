import { expect, test } from '@playwright/test'
import { serveNode, stubNode } from '../support/stub-node'

test.describe('node lifecycle controls', () => {
  test('running node offers Restart and a Stop that explains the hold file', async ({ page }) => {
    const { actions } = await serveNode(page, stubNode({ node_id: 'ui-running' }))
    await page.goto('/nodes/ui-running')

    await expect(page.getByTestId('lifecycle-state')).toHaveText('healthy')
    await expect(page.getByRole('button', { name: 'Restart', exact: true })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Start', exact: true })).toHaveCount(0)

    await page.getByRole('button', { name: 'Stop', exact: true }).click()
    const dialog = page.getByRole('dialog', { name: 'Stop Validator' })
    await expect(dialog).toContainText('.pillar/<unit>.hold')
    await expect(dialog).toContainText('will not auto-restart')
    await dialog.getByRole('button', { name: 'Stop' }).click()

    await expect(page.getByText('Stop command sent.')).toBeVisible()
    expect(actions).toEqual(['stop'])
  })

  test('cancelling the Stop dialog sends nothing', async ({ page }) => {
    const { actions } = await serveNode(page, stubNode({ node_id: 'ui-cancel' }))
    await page.goto('/nodes/ui-cancel')

    await page.getByRole('button', { name: 'Stop', exact: true }).click()
    await page.getByRole('dialog').getByRole('button', { name: 'Cancel' }).click()
    await expect(page.getByRole('dialog')).toHaveCount(0)
    expect(actions).toEqual([])
  })

  test('stopped node shows a stopped badge and Start instead of Restart', async ({ page }) => {
    const node = stubNode({ node_id: 'ui-stopped', lifecycle_state: 'stopped' })
    node.live_status = { ...node.live_status!, state: 'stopped', healthy: false }
    const { actions } = await serveNode(page, node)
    await page.goto('/nodes/ui-stopped')

    await expect(page.getByTestId('lifecycle-state')).toHaveText('stopped')
    await expect(page.getByRole('button', { name: 'Restart', exact: true })).toHaveCount(0)

    await page.getByRole('button', { name: 'Start', exact: true }).click()
    const dialog = page.getByRole('dialog', { name: 'Start Validator' })
    await expect(dialog).toContainText('clears the stop hold')
    await dialog.getByRole('button', { name: 'Start' }).click()

    await expect(page.getByText('Start command sent.')).toBeVisible()
    // Start reuses the restart endpoint, whose script clears the hold file.
    expect(actions).toEqual(['restart'])
  })

  test('lifecycle buttons are disabled while a deployment is running', async ({ page }) => {
    await serveNode(page, stubNode({
      node_id: 'ui-deploying',
      lifecycle_state: 'provisioning',
      active_script: { script_id: 's1', description: 'Provision surfpool', initiated_at: Math.floor(Date.now() / 1000) },
    }))
    await page.goto('/nodes/ui-deploying')

    for (const name of ['Restart', 'Stop', 'Recover']) {
      await expect(page.getByRole('button', { name, exact: true })).toBeDisabled()
    }
  })
})
