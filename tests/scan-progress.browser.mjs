import { tmpdir } from 'node:os';
import { join } from 'node:path';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
import assert from 'node:assert/strict';
const browser = await chromium.launch({headless:true});
try {
 const page = await browser.newPage({ viewport:{width:1280,height:800} });
 const errors=[]; page.on('pageerror',e=>errors.push(e.message));
 await page.goto(process.env.TIDY_URL || 'http://127.0.0.1:1420');
 await page.evaluate(async()=>{
  const {mockIPC}=await import('/node_modules/@tauri-apps/api/mocks.js');
  window.calls=[];
  mockIPC((cmd,args)=>{
   if(cmd==='plugin:dialog|open') return '/photos';
   if(cmd==='cancel_scan') return;
   return new Promise((resolve,reject)=>window.calls.push({cmd,args,reject,resolve:(value)=>{
    if(cmd==='scan_directory' && Array.isArray(value)) { args.onFiles.onmessage({files:value,total:value.length,done:true});resolve(); }
    else resolve(value);
   }}));
  });
 });
 await page.getByRole('button',{name:'📁 打开目录'}).click();
 await page.getByRole('progressbar',{name:'正在查找媒体文件…'}).waitFor();
 assert.equal(await page.locator('progress').getAttribute('value'),null);
 await page.evaluate(()=>window.calls.at(-1).args.onProgress.onmessage({stage:'metadata',current:2500,total:10000}));
 await page.getByText('2500 / 10000（25%）').waitFor();
 await page.screenshot({path:join(tmpdir(), 'tidy-scan-desktop.png')});
 await page.setViewportSize({width:390,height:844});
 await page.screenshot({path:join(tmpdir(), 'tidy-scan-narrow.png')});
 const bounds=await page.locator('.preload .card').boundingBox();
 assert.ok(bounds.x>=0 && bounds.x+bounds.width<=390);
 await page.getByRole('button',{name:'取消扫描'}).focus();
 await page.keyboard.press('Enter');
 await page.getByRole('button',{name:'📁 打开目录'}).waitFor();
 await page.evaluate(()=>{const c=window.calls.find(c=>c.cmd==='scan_directory');c.args.onProgress.onmessage({stage:'pairing',current:10000,total:10000});c.resolve([])});
 await page.getByRole('button',{name:'📁 打开目录'}).click();
 await page.evaluate(()=>window.calls.at(-1).reject('无法读取目录'));
 await page.getByRole('alert').filter({hasText:'无法读取目录'}).waitFor();
 await page.setViewportSize({width:1280,height:800});
 await page.getByRole('button',{name:'📁 打开目录'}).click();
 await page.evaluate(()=>window.calls.at(-1).resolve([]));
 await page.locator('.titlebar').getByRole('button',{name:'刷新目录',exact:true}).waitFor();
 await page.locator('.titlebar').getByRole('button',{name:'刷新目录',exact:true}).click();
 await page.evaluate(()=>window.calls.at(-1).args.onProgress.onmessage({stage:'pairing',current:8000,total:10000}));
 await page.getByText('8000 / 10000（80%）').waitFor();
 await page.screenshot({path:join(tmpdir(), 'tidy-scan-refresh.png')});
 await page.getByRole('button',{name:'取消刷新',exact:true}).click();
 assert.equal(await page.getByRole('progressbar').count(),0);
 assert.deepEqual(errors,[]);
 console.log('PASS: browser IPC mock: unknown/determinate progress, keyboard cancel, stale response, failure display, retry, refresh progress/cancel; desktop/narrow screenshots; no page errors.');
} finally { await browser.close(); }
