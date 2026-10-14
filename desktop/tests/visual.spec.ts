import { _electron as electron, expect, test } from '@playwright/test'

test('app shell visual regression across light desktop and dark mobile', async () => {
  const app = await electron.launch({ args: [process.cwd()] })
  const page = await app.firstWindow()

  await page.waitForLoadState('domcontentloaded')
  await page.getByText('Runtime').waitFor()

  await page.emulateMedia({ colorScheme: 'light' })
  await page.setViewportSize({ width: 1280, height: 800 })
  await expect(page).toHaveScreenshot('shell-light-desktop.png')

  await page.emulateMedia({ colorScheme: 'dark' })
  await page.setViewportSize({ width: 375, height: 800 })
  await expect(page).toHaveScreenshot('shell-dark-mobile.png')

  await app.close()
})
