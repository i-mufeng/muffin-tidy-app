import { tmpdir } from 'node:os';
import { join } from 'node:path';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
import assert from 'node:assert/strict';
const browser=await chromium.launch({headless:true});
try {
 const page=await browser.newPage({viewport:{width:1280,height:800}});
 const errors=[]; page.on('pageerror',e=>errors.push(e.message));
 await page.goto(process.env.TIDY_URL || 'http://127.0.0.1:1420');
 await page.evaluate(async()=>{
  const {mockIPC}=await import('/node_modules/@tauri-apps/api/mocks.js');
  window.counts={thumbnail:0,preview:0,preload:0};
  window.maxTickGap=0;let last=performance.now();window.tick=setInterval(()=>{const now=performance.now();window.maxTickGap=Math.max(window.maxTickGap,now-last);last=now},16);
  const pixel='data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jf1sAAAAASUVORK5CYII=';
  mockIPC(async(cmd,args)=>{
   if(cmd==='plugin:dialog|open') return '/large-library';
   if(cmd==='scan_directory') {
    for(let offset=0;offset<20000;offset+=256){
     const files=Array.from({length:Math.min(256,20000-offset)},(_,j)=>({id:String(offset+j),source_path:`/large-library/photo-${offset+j}.jpg`,media_type:'img',capture_time:'2026-09-29 12:00:00',file_size:6*1024*1024,live_type:null,video_path:null,duration:null,exif_info:{}}));
     args.onFiles.onmessage({files,total:20000,done:offset+256>=20000});
    }
    return;
   }
   if(cmd==='preload_thumbnails'){window.counts.preload=args.paths.length;args.onProgress.onmessage({done:args.paths.length,total:args.paths.length});return;}
   if(cmd==='get_thumbnail') {window.counts.thumbnail++;await new Promise(r=>setTimeout(r,20));return pixel;}
   if(cmd==='get_preview') {window.counts.preview++;await new Promise(r=>setTimeout(r,20));return pixel;}
  });
 });
 await page.getByRole('button',{name:'📁 打开目录'}).click();
 await page.getByText('20000 个媒体').waitFor();
 await page.locator('.thumb-card').first().waitFor();
 // User interactions with the virtualized grid; current item and large viewer.
 await page.locator('.thumb-card').first().click();
 await page.keyboard.press('Space');
 await page.locator('.thumb-card').first().dblclick();
 await page.keyboard.press('ArrowRight');
 await page.keyboard.press('ArrowRight');
 await page.keyboard.press('Escape');
 await page.mouse.move(400,400);
 for(let i=0;i<12;i++) await page.mouse.wheel(0,1400);
 await page.waitForFunction(() => [...document.querySelectorAll('.thumb-card')].every(card => card.querySelector('img')?.naturalWidth > 0));
 await page.screenshot({path:join(tmpdir(), 'tidy-library-20k.png')});
 const stats=await page.evaluate(()=>{clearInterval(window.tick);return {...window.counts,maxTickGapMs:Math.round(window.maxTickGap),cards:document.querySelectorAll('.thumb-card').length,rawImages:[...document.images].filter(i=>i.src.startsWith('asset:')||i.src.includes('asset.localhost')).length}});
 assert.equal(stats.preload,48);assert.ok(stats.cards<200);assert.ok(stats.thumbnail<500);assert.equal(stats.rawImages,0);assert.deepEqual(errors,[]);
 console.log(JSON.stringify({result:'PASS',files:20000,logicalGiB:20000*6/1024, ...stats,scope:'Chromium with mocked IPC, not native disk or image decoding'},null,2));
}finally{await browser.close()}
