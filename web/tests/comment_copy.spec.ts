import { expect, test } from './wview';

test.use({ permissions: ['clipboard-read', 'clipboard-write'] });

test('blob page renders with the embedded bundle', async ({ page, server }) => {
  await page.goto(`${server.baseURL}/blob/f.py`);
  await expect(page.locator('#L1')).toBeVisible();
  const kind = await page.evaluate(
    () => typeof (window as { wviewComments?: { fmtOne?: unknown } }).wviewComments?.fmtOne,
  );
  expect(kind).toBe('function');
});

test('comment on a line, persist, and copy in the export format', async ({ page, server }) => {
  await page.goto(`${server.baseURL}/blob/f.py`);

  // Anchor line 1 via its line number, then open the comment editor.
  await page.click('#L1 a');
  await page.keyboard.press('c');
  const editor = page.locator('tr.cedit textarea');
  await editor.fill('please add a docstring');
  await editor.press('Control+Enter');
  await expect(page.locator('tr.cmt .body')).toHaveText('please add a docstring');

  // Survives a reload (localStorage-backed).
  await page.reload();
  await expect(page.locator('tr.cmt .body')).toHaveText('please add a docstring');

  // Copy produces the exact export format.
  await page.locator('tr.cmt button', { hasText: 'copy' }).first().click();
  const clipboard = await page.evaluate(() => navigator.clipboard.readText());
  expect(clipboard).toBe('f.py:1:\n> def foo():\nplease add a docstring');
});
