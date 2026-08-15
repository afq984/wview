import { expect, test } from './wview';

test('clicking a line marker anchors without scrolling', async ({ page, server }) => {
  await page.goto(`${server.baseURL}/blob/many.py`);

  // Park line 100 mid-viewport with the page scrolled.
  await page.evaluate(() => {
    const row = document.getElementById('L100')!;
    scrollTo(0, row.getBoundingClientRect().top + scrollY - 300);
  });
  const before = await page.evaluate(() => scrollY);
  expect(before).toBeGreaterThan(0);

  await page.click('#L100 a');
  const after = await page.evaluate(() => scrollY);
  expect(after).toBe(before);
  expect(new URL(page.url()).hash).toBe('#L100');
  await expect(page.locator('#L100')).toHaveClass(/ltarget/);

  // The anchored line is what c comments on.
  await page.keyboard.press('c');
  const editor = page.locator('tr.cedit textarea');
  await expect(editor).toBeFocused();
  await expect(editor).toHaveAttribute('placeholder', /L100/);
});

test('loading a line permalink still scrolls to the line', async ({ page, server }) => {
  await page.goto(`${server.baseURL}/blob/many.py#L150`);
  await expect(page.locator('#L150')).toBeInViewport();
  expect(await page.evaluate(() => scrollY)).toBeGreaterThan(0);
  await expect(page.locator('#L150')).toHaveClass(/ltarget/);
});
