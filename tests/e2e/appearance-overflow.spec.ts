import {test,expect,type Page} from '@playwright/test';

async function settled(page:Page){await page.evaluate(()=>document.fonts.ready);await page.waitForFunction(()=>Array.from(document.querySelectorAll('[data-rubber]')).every(root=>{const selected=root.querySelector('[aria-current="page"]'),layer=root.querySelector('[data-rubber-indicator]');if(!selected||!layer)return true;const a=selected.getBoundingClientRect(),b=layer.getBoundingClientRect();return Math.abs(a.y-b.y)<.5&&Math.abs(a.x-b.x)<.5;}));}

for(const size of [{width:1120,height:760,scale:1},{width:960,height:640,scale:1.5}])test(`theme persistence, shared windows and card overflow ${size.width}`,async({browser})=>{
 const context=await browser.newContext({viewport:size,deviceScaleFactor:size.scale}),page=await context.newPage();
 const errors:string[]=[];page.on('pageerror',error=>errors.push(error.message));
 await page.goto('/');await page.getByRole('button',{name:'设置',exact:true}).click();
 const dialog=page.getByRole('dialog',{name:'设置',exact:true});
 await dialog.getByRole('button',{name:'外观',exact:true}).click();
 for(const [id,name] of [['light','浅色'],['graphite','石墨银灰']]){
  await dialog.getByRole('button',{name:name!,exact:true}).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme',id!);
  await expect(dialog.getByRole('button',{name:name!,exact:true})).toHaveAttribute('aria-pressed','true');
  await expect.poll(()=>page.evaluate(()=>JSON.parse(localStorage.getItem('sayweave.appearance')??'{"themeId":"light"}').themeId)).toBe(id);
  await settled(page);await page.screenshot({path:`.local/appearance-input/ui/${id}-appearance-${size.width}.png`});
  if(size.width===1120){
   for(const tab of ['设备与快捷键','模型','关于']){
    await dialog.getByRole('button',{name:tab,exact:true}).click();await settled(page);
    await page.screenshot({path:`.local/appearance-input/ui/${id}-${tab}.png`});
   }
   await dialog.getByRole('button',{name:'外观',exact:true}).click();
  }
  await dialog.getByRole('button',{name:'关闭',exact:true}).click();
  const area=page.locator('section[aria-labelledby="usage-title"]'),face=area.locator(':scope > div').nth(1),back=area.locator(':scope > div').first();
  const box=(await area.boundingBox())!,backBefore=await back.boundingBox();
  for(const [x,y] of [[.01,.01],[.99,.01],[.99,.99],[.01,.99]]){
   await page.mouse.move(box.x+box.width*x!,box.y+box.height*y!,{steps:6});await expect(face).toHaveAttribute('data-tilting','true');
   expect(await page.locator('main').first().evaluate(el=>({width:el.clientWidth,scroll:el.scrollWidth,overflow:getComputedStyle(el).overflowX}))).toMatchObject({overflow:'hidden'});
   expect(await page.locator('main').first().evaluate(el=>el.scrollWidth<=el.clientWidth)).toBe(true);
   expect(await back.boundingBox()).toEqual(backBefore);
  }
  await area.getByRole('button').click();
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  await page.mouse.move(5,5);await expect(face).not.toHaveAttribute('data-tilting');
  await expect.poll(()=>face.evaluate(el=>Math.abs(new DOMMatrix(getComputedStyle(el).transform).m42))).toBeLessThan(.01);await settled(page);
  await page.screenshot({path:`.local/appearance-input/ui/${id}-home-${size.width}.png`});
  await page.goto('/#/history');await settled(page);await page.screenshot({path:`.local/appearance-input/ui/${id}-history-${size.width}.png`});
  await page.goto('/');await expect(page.locator('html')).toHaveAttribute('data-theme',id!);
  await page.getByRole('button',{name:'设置',exact:true}).click();await dialog.getByRole('button',{name:'外观',exact:true}).click();
 }
 // A second actual page uses the same AppearancePort and applies changes while
 // the main window saves; no test-only theme setter is involved.
 const overlay=await context.newPage();await overlay.goto('/?surface=overlay&voiceDemo=1');
 await expect(overlay.locator('html')).toHaveAttribute('data-theme','graphite');
 await overlay.getByRole('button').first().click();
 await expect(overlay.locator('[data-phase="recording"]')).toBeVisible();
 await overlay.screenshot({path:`.local/appearance-input/ui/graphite-voice-${size.width}.png`});
 await dialog.getByRole('button',{name:'浅色',exact:true}).click();
 await expect(overlay.locator('html')).toHaveAttribute('data-theme','light');
 await overlay.screenshot({path:`.local/appearance-input/ui/light-voice-${size.width}.png`});
 await page.emulateMedia({reducedMotion:'reduce'});await dialog.getByRole('button',{name:'关闭',exact:true}).click();
 const face=page.locator('section[aria-labelledby="usage-title"] > div').nth(1);await face.hover();await expect(face).toHaveCSS('transform','none');
 expect(errors).toEqual([]);await context.close();
});

test('success and unchecked control contrast roles',async({page})=>{
 await page.goto('/');
 for(const [id,name] of [['light','浅色'],['graphite','石墨银灰']]){
  await page.getByRole('button',{name:'设置',exact:true}).click();
  const dialog=page.getByRole('dialog',{name:'设置',exact:true});
  await dialog.getByRole('button',{name:'外观',exact:true}).click();
  await dialog.getByRole('button',{name:name!,exact:true}).click();
  await dialog.getByRole('button',{name:'关闭',exact:true}).click();
  await page.goto('/#/dev/components');
  await page.getByRole('button',{name:'测试演示连接',exact:true}).click();
  const status=page.getByRole('status').filter({hasText:'演示连接成功'});
  await expect(status).toBeVisible();
  await expect(status.locator('svg')).toHaveAttribute('stroke','var(--on-signal)');
  await status.screenshot({path:`.local/appearance-input/ui/${id}-success.png`});
  const toggle=page.getByRole('switch',{name:'开关',exact:true});
  await expect(toggle).toHaveAttribute('data-state','unchecked');
  await toggle.screenshot({path:`.local/appearance-input/ui/${id}-switch-off.png`});
  await page.goto('/');
 }
});
