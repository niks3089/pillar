import path from 'node:path'
import { expect, type Page } from '@playwright/test'
import { e2eEnv } from '../env'

export const authStatePath = path.join(import.meta.dirname, '../.auth/admin.json')

export async function loginThroughUi(page: Page, password = e2eEnv.adminPassword): Promise<void> {
  await page.goto('/')
  await page.getByLabel('Username').fill(e2eEnv.adminUsername)
  await page.getByLabel('Password').fill(password)
  await page.getByRole('button', { name: 'Sign in' }).click()
}

export async function expectLoggedIn(page: Page): Promise<void> {
  await expect(page.getByLabel('Password')).toHaveCount(0)
}
