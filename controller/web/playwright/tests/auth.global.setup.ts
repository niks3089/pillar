import { test as setup } from '@playwright/test'
import { authStatePath, expectLoggedIn, loginThroughUi } from '../support/auth'

setup('log in as admin', async ({ page }) => {
  await loginThroughUi(page)
  await expectLoggedIn(page)
  await page.context().storageState({ path: authStatePath })
})
