import { expect, test } from '@playwright/test'

test('operations guide explains what runs on the node and how Stop holds', async ({ page }) => {
  await page.goto('/docs')
  await expect(page.getByRole('heading', { name: 'What Pillar runs on your node' })).toBeVisible()
  await expect(page.getByText('pillar-agent.service').first()).toBeVisible()
  await expect(page.getByText(/\.pillar\/<unit>\.hold/).first()).toBeVisible()
  await expect(page.getByRole('cell', { name: /write hold file/ })).toBeVisible()
  await expect(page.getByText('Seeing exactly what runs.')).toBeVisible()
  await expect(page.getByText(/\[cmd\] <command>/)).toBeVisible()
})
