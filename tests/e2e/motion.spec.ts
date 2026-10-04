import { test, expect } from '@playwright/test';
import { writeFile } from 'node:fs/promises';

test('password geometry/caret and async examples have no service side effects', async ({ page }) => {
  const requests: string[] = []; page.on('request', req => { if (req.url().startsWith('http') && !req.url().startsWith('http://127.0.0.1:5173')) requests.push(req.url()); });
  await page.goto('/#/dev/components');
  const input = page.getByLabel('演示密钥', { exact: true }); const before = await input.boundingBox();
  await input.evaluate((node: HTMLInputElement) => node.setSelectionRange(2, 5));
  for (let i = 0; i < 4; i++) { const toggle = page.getByRole('button', { name: i % 2 ? '隐藏内容' : '显示内容', exact: true }); await toggle.click(); await expect(page.getByRole('button', { name: i % 2 ? '显示内容' : '隐藏内容', exact: true })).toBeFocused(); expect(await input.boundingBox()).toEqual(before); expect(await input.evaluate((node: HTMLInputElement) => [node.value, node.selectionStart, node.selectionEnd])).toEqual(['demo-only', 2, 5]); }
  const copies = page.getByRole('button', { name: '复制', exact: true });
  await copies.first().click(); await expect(page.getByRole('button', { name: '已复制', exact: true })).toBeVisible();
  await copies.last().click(); await expect(page.getByRole('button', { name: '复制失败，重试', exact: true })).toBeVisible();
  await page.getByRole('button', { name: '测试演示连接', exact: true }).click(); await expect(page.getByText('演示连接成功', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: '模拟连接失败', exact: true }).click(); await expect(page.getByText('演示连接失败，请重试', { exact: true })).toBeVisible();
  await page.screenshot({ path: 'tests/visual/motion-feedback.png', fullPage: true });
  expect(requests).toEqual([]);
});

test('native visibility snapshot pauses all development feedback without requiring document.hidden', async ({ page }) => {
  await page.goto('/#/dev/components');
  await page.getByRole('button', { name: '测试演示连接', exact: true }).click();
  await page.getByRole('button', { name: '隐藏到托盘', exact: true }).click();
  await expect.poll(() => page.locator('[data-icon][data-motion="active"]').count()).toBe(0);
  expect(await page.evaluate(() => document.hidden)).toBe(false);
  await expect.poll(() => page.evaluate(() => document.getAnimations().filter(x => x.playState === 'running').length)).toBe(0);
  await expect(page.getByText('演示连接成功', { exact: true })).toBeVisible();
});

for (const reducedMotion of ['no-preference', 'reduce'] as const) test(`voice state geometry, SVG continuity and visibility: ${reducedMotion}`, async ({ page }) => {
  await page.emulateMedia({ reducedMotion }); await page.setViewportSize({ width: 1120, height: 760 }); await page.goto('/#/dev/components');
  const control = page.locator('[data-phase]');
  await page.getByRole('button', { name: '开始预设演示' }).click(); await expect(page.locator('[data-phase="recording"]')).toBeVisible();
  await expect.poll(async () => (await control.boundingBox())?.width).toBe(216);
  await expect.poll(async () => control.getAttribute('data-motion')).toBe(reducedMotion === 'reduce' ? 'static' : 'active');
  const frames = await page.evaluate(async () => {
    const intervals: number[] = []; let last = 0;
    await new Promise<void>(resolve => { const sample = (now: number) => { if (last) intervals.push(now - last); last = now; if (intervals.length === 90) resolve(); else requestAnimationFrame(sample); }; requestAnimationFrame(sample); });
    const sorted = [...intervals].sort((a, b) => a - b);
    return { frames: intervals.length, p95Ms: sorted[Math.floor(sorted.length * .95)], maxMs: sorted.at(-1), over50Ms: intervals.filter(x => x > 50).length, activeAnimations: document.getAnimations().filter(x => x.playState === 'running').length, devicePixelRatio };
  });
  await writeFile(`tests/visual/motion-performance-${reducedMotion}.json`, JSON.stringify({ environment: 'Playwright Chromium; not native FPS', reducedMotion, ...frames }, null, 2) + '\n');
  if (reducedMotion === 'reduce') expect(frames.activeAnimations).toBe(0);
  await page.screenshot({ path: `tests/visual/voice-recording-${reducedMotion}.png` });
  await page.getByRole('button', { name: '完成演示' }).click();
  await page.locator('[data-status-glyph] svg').evaluate(node => node.setAttribute('data-stable-test', 'same'));
  await expect(page.locator('[data-phase="completed"]')).toBeVisible();
  await expect(page.locator('[data-status-glyph] svg')).toHaveAttribute('data-stable-test', 'same');
  await page.screenshot({ path: `tests/visual/voice-completed-${reducedMotion}.png` });
  await expect(page.locator('[data-phase="idle"]')).toBeVisible();
  await page.getByRole('button', { name: '开始预设演示' }).click(); await page.getByRole('button', { name: '取消演示' }).click();
  await expect(page.locator('[data-phase="cancelled"]')).toBeVisible();
  await page.getByRole('button', { name: '开始预设演示' }).click(); await expect(page.locator('[data-phase="recording"]')).toBeVisible();
  await page.getByRole('button', { name: '设置', exact: true }).click(); await expect(control).toHaveAttribute('data-motion', 'static');
  await page.keyboard.press('Escape'); await expect(control).toHaveAttribute('data-motion', reducedMotion === 'reduce' ? 'static' : 'active');
});
