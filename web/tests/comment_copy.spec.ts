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
  await expect(editor).toBeFocused();
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

async function addComment(page: import('@playwright/test').Page, body: string) {
  await page.click('#L1 a');
  await page.keyboard.press('c');
  const editor = page.locator('tr.cedit textarea');
  await editor.fill(body);
  await editor.press('Control+Enter');
  await expect(page.locator('tr.cmt .body')).toHaveText(body);
}

test('edit replaces the comment display until save or cancel', async ({ page, server }) => {
  await page.goto(`${server.baseURL}/blob/f.py`);
  await addComment(page, 'original');

  // Cancel restores the untouched comment.
  await page.locator('tr.cmt button', { hasText: 'edit' }).click();
  await expect(page.locator('tr.cmt')).toBeHidden();
  const editor = page.locator('tr.cedit textarea');
  await expect(editor).toHaveValue('original');
  await editor.press('Escape');
  await expect(page.locator('tr.cmt .body')).toHaveText('original');

  // Save swaps in the new body.
  await page.locator('tr.cmt button', { hasText: 'edit' }).click();
  await editor.fill('revised');
  await editor.press('Control+Enter');
  await expect(page.locator('tr.cmt .body')).toHaveText('revised');
});

test('deleting another comment while editing keeps the editor swapped in', async ({ page, server }) => {
  await page.goto(`${server.baseURL}/blob/f.py`);
  await addComment(page, 'first');
  await page.click('#L2 a');
  await page.keyboard.press('c');
  const ta = page.locator('tr.cedit textarea');
  await ta.fill('second');
  await ta.press('Control+Enter');
  await expect(page.locator('tr.cmt')).toHaveCount(2);

  await page.locator('tr.cmt', { hasText: 'first' }).locator('button', { hasText: 'edit' }).click();
  await ta.fill('first revised');
  await page.locator('tr.cmt', { hasText: 'second' }).locator('button', { hasText: 'delete' }).click();

  // The edited comment's fresh display row stays swapped out for the editor,
  // and the draft survives.
  await expect(page.locator('tr.cmt:visible')).toHaveCount(0);
  await expect(ta).toHaveValue('first revised');
  await ta.press('Control+Enter');
  await expect(page.locator('tr.cmt .body')).toHaveText('first revised');
  await expect(page.locator('tr.cmt')).toHaveCount(1);
});

test('c on a drag selection opens a focused editor', async ({ page, server }) => {
  await page.goto(`${server.baseURL}/blob/f.py`);
  const from = await page.locator('#L1 td.c').boundingBox();
  const to = await page.locator('#L2 td.c').boundingBox();
  if (!from || !to) throw new Error('missing line boxes');
  await page.mouse.move(from.x + 2, from.y + from.height / 2);
  await page.mouse.down();
  await page.mouse.move(to.x + to.width / 2, to.y + to.height / 2, { steps: 8 });
  await page.mouse.up();
  await page.keyboard.press('c');

  const editor = page.locator('tr.cedit textarea');
  await expect(editor).toBeFocused();
  // Type through the keyboard: if focus were elsewhere, the 'c' in
  // "comment" would re-trigger the shortcut instead of landing here.
  await page.keyboard.type('range comment');
  await expect(editor).toHaveValue('range comment');
  await page.keyboard.press('Control+Enter');
  await expect(page.locator('tr.cmt .body')).toHaveText('range comment');

  await page.locator('tr.cmt button', { hasText: 'copy' }).click();
  const clip = await page.evaluate(() => navigator.clipboard.readText());
  expect(clip).toBe('f.py:1:\n> def foo():\n>     pass\nrange comment');
});

test('text selection skips comment UI', async ({ page, server }) => {
  await page.goto(`${server.baseURL}/blob/f.py`);
  await addComment(page, 'do not select me');

  // Drag from the start of L1's code across the comment box into L2.
  const from = await page.locator('#L1 td.c').boundingBox();
  const to = await page.locator('#L2 td.c').boundingBox();
  if (!from || !to) throw new Error('missing line boxes');
  await page.mouse.move(from.x + 2, from.y + from.height / 2);
  await page.mouse.down();
  await page.mouse.move(to.x + to.width / 2, to.y + to.height / 2, { steps: 8 });
  await page.mouse.up();

  // Chrome's Selection.toString() ignores user-select:none, but what lands
  // on the clipboard — the behavior that matters — must be code only.
  await page.keyboard.press('Control+c');
  const copied = await page.evaluate(() => navigator.clipboard.readText());
  expect(copied).toContain('def foo():');
  expect(copied).toContain('pas');
  expect(copied).not.toContain('do not select me');
  expect(copied).not.toContain('delete');
});
