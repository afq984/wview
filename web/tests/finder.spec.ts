import { expect, test } from './wview';

test('finder matches server-side and navigates to the file', async ({ page, server }) => {
  await page.goto(`${server.baseURL}/`);
  await page.fill('#q', 'librs');
  const link = page.locator('#files a', { hasText: 'sub/lib.rs' });
  await expect(link).toBeVisible();
  await link.click();
  await expect(page.locator('td.c').first()).toContainText('fn main');
});

test('empty query lists files', async ({ page, server }) => {
  await page.goto(`${server.baseURL}/`);
  await expect(page.locator('#files a', { hasText: 'f.py' })).toBeVisible();
});
