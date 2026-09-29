import assert from 'node:assert/strict';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  await page.goto(process.env.TIDY_URL || 'http://127.0.0.1:1420');
  await page.evaluate(async () => {
    const { mockIPC } = await import('/node_modules/@tauri-apps/api/mocks.js');
    window.calls = [];
    window.supported = false;
    mockIPC((cmd, args) => {
      if (cmd === 'pd_supported') { if (window.supported === 'error') throw new Error('平台探测失败'); return window.supported; }
      if (cmd === 'acknowledge_scan_batch') return;
      if (cmd.startsWith('plugin:window|')) return;
      return new Promise((resolve, reject) => window.calls.push({ cmd, args, resolve, reject }));
    });
  });
  const open = page.getByRole('button', { name: '📱 从手机导入' });
  await open.click();
  const dialog = page.getByRole('dialog', { name: '📱 从手机导入' });
  await page.getByText('手机 USB 直连目前仅支持 Windows。', { exact: false }).waitFor();
  assert.equal(await page.evaluate(() => window.calls.length), 0);
  await page.keyboard.press('Escape');
  await dialog.waitFor({ state: 'hidden' });
  assert.equal(await open.evaluate(el => el === document.activeElement), true);
  await page.evaluate(() => window.supported = true);
  await open.click();
  await page.waitForFunction(() => window.calls.some(c => c.cmd === 'pd_browse'));
  // A late device-list response after closing must not replace a later session.
  await page.keyboard.press('Escape');
  await dialog.waitFor({ state: 'hidden' });
  await open.click();
  await page.waitForFunction(() => window.calls.filter(c => c.cmd === 'pd_browse').length === 2);
  await page.evaluate(() => {
    const requests = window.calls.filter(c => c.cmd === 'pd_browse');
    requests[1].resolve([{ id: 'phone', name: '测试手机', is_folder: true, is_filesystem: false }]);
    requests[0].resolve([{ id: 'stale', name: '过期设备', is_folder: true, is_filesystem: false }]);
  });
  await page.getByText('测试手机', { exact: true }).waitFor();
  assert.equal(await page.getByText('过期设备').count(), 0);
  await dialog.focus();
  await page.keyboard.press('Shift+Tab');
  assert.equal(await dialog.getByRole('button', { name: '取消', exact: true }).evaluate(el => el === document.activeElement), true);
  await page.keyboard.press('Tab');
  assert.equal(await dialog.getByRole('button', { name: '关闭手机导入' }).evaluate(el => el === document.activeElement), true);
  await dialog.getByRole('button', { name: '导入', exact: true }).click();
  await page.waitForFunction(() => window.calls.some(c => c.args?.parentId === 'phone'));
  await page.evaluate(() => window.calls.find(c => c.args?.parentId === 'phone').resolve([{ id: 'storage', name: '内部存储', is_folder: true }]));
  await page.getByRole('progressbar', { name: '手机导入进度' }).waitFor();
  assert.equal(await page.getByRole('progressbar').getAttribute('aria-valuenow'), null);
  await page.waitForFunction(() => window.calls.some(c => c.cmd === 'pd_import'));
  await page.evaluate(() => window.calls.find(c => c.cmd === 'pd_import').args.onProgress.onmessage({ phase: 'copying', done_files: 10, total_files: 10, done_bytes: 2048 }));
  assert.equal(await page.getByRole('progressbar').getAttribute('aria-valuenow'), '99');
  await page.keyboard.press('Escape');
  assert.equal(await dialog.count(), 1);
  await page.getByRole('button', { name: '取消导入', exact: true }).click();
  await page.waitForFunction(() => window.calls.some(c => c.cmd === 'pd_cancel_import'));
  await page.evaluate(() => window.calls.find(c => c.cmd === 'pd_cancel_import').reject('设备忙'));
  await page.getByRole('alert').filter({ hasText: '取消失败，请重试：设备忙' }).waitFor();
  assert.equal(await page.getByRole('button', { name: '取消导入', exact: true }).isEnabled(), true);
  await page.screenshot({ path: join(tmpdir(), 'tidy-phone-desktop.png') });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: join(tmpdir(), 'tidy-phone-narrow.png') });
  const bounds = await dialog.boundingBox();
  assert.ok(bounds.x >= 0 && bounds.x + bounds.width <= 390);
  await page.setViewportSize({ width: 320, height: 640 });
  const small = await dialog.boundingBox();
  assert.ok(small.x >= 0 && small.x + small.width <= 320);
  await page.getByRole('button', { name: '取消导入', exact: true }).click();
  await page.waitForFunction(() => window.calls.filter(c => c.cmd === 'pd_cancel_import').length === 2);
  await page.evaluate(() => {
    window.calls.filter(c => c.cmd === 'pd_cancel_import')[1].resolve();
    window.calls.find(c => c.cmd === 'pd_import').resolve({ cancelled: true, file_count: 0, temp_dir: '', elapsed_ms: 0 });
  });
  await page.getByRole('button', { name: '导入', exact: true }).waitFor();
  await page.keyboard.press('Escape');
  await dialog.waitFor({ state: 'hidden' });
  // Platform detection failures remain retryable.
  await page.evaluate(() => { window.supported = 'error'; window.calls = []; });
  await open.click();
  await page.getByRole('alert').filter({ hasText: '平台探测失败' }).waitFor();
  assert.equal(await page.getByRole('button', { name: '刷新设备' }).isEnabled(), true);
  await page.evaluate(() => window.supported = true);
  await page.getByRole('button', { name: '刷新设备' }).click();
  await page.waitForFunction(() => window.calls.some(c => c.cmd === 'pd_browse'));
  await page.evaluate(() => window.calls.find(c => c.cmd === 'pd_browse').resolve([{ id: 'phone', name: '测试手机', is_folder: true }]));
  await page.getByRole('button', { name: '导入', exact: true }).click();
  await page.waitForFunction(() => window.calls.some(c => c.args?.parentId === 'phone'));
  await page.evaluate(() => window.calls.find(c => c.args?.parentId === 'phone').resolve([{ id: 'storage', name: '内部存储', is_folder: true }]));
  await page.waitForFunction(() => window.calls.some(c => c.cmd === 'pd_import'));
  await page.evaluate(() => window.calls.find(c => c.cmd === 'pd_import').resolve({ cancelled: false, file_count: 1, temp_dir: '/tmp/muffin-phone-test', elapsed_ms: 10 }));
  await dialog.waitFor({ state: 'hidden' });
  await page.getByRole('progressbar', { name: '正在查找媒体文件…' }).waitFor();
  await page.waitForFunction(() => window.calls.some(c => c.cmd === 'scan_directory'));
  assert.equal(await page.evaluate(() => window.calls.find(c => c.cmd === 'scan_directory').args.path), '/tmp/muffin-phone-test');
  await page.evaluate(() => {
    const scan = window.calls.find(c => c.cmd === 'scan_directory');
    scan.args.onFiles.onmessage({ files: [], total: 0, done: true, generation: 1, sequence: 0 });
    scan.resolve();
  });
  await page.locator('.titlebar').getByRole('button', { name: '刷新目录', exact: true }).waitFor();
  assert.deepEqual(errors, []);
  console.log('PASS: browser IPC mocks: unsupported platform, late browse response, focus trap/restore, unknown progress, no premature 100%, cancel failure/retry, cancellation return, platform detection retry, successful import-to-scan-to-workspace; desktop/390px screenshots and 320px bounds; no page errors.');
} finally { await browser.close(); }
