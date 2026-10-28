import { expect, test } from '@playwright/test'
import { expectLoggedIn, loginThroughUi } from '../support/auth'

test('wrong password is rejected with a visible error', async ({ page }) => {
  await loginThroughUi(page, 'definitely-not-the-password')
  await expect(page.getByText(/invalid username or password/i)).toBeVisible()
  await expect(page.getByLabel('Password')).toBeVisible()
})

test('admin can sign in and lands on the fleet overview', async ({ page }) => {
  await loginThroughUi(page)
  await expectLoggedIn(page)
  await expect(page).toHaveURL(/\/$/)
})
