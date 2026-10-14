import {test,expect,type Page} from '@playwright/test';

async function settled(page:Page){
 await page.waitForFunction(()=>Array.from(document.querySelectorAll('[data-rubber]')).every(root=>{
  const selected=root.querySelector('[aria-current="page"],[data-choice-item][data-current="true"]'),layer=root.querySelector('[data-rubber-indicator]');
  if(!selected||!layer)return true;const a=selected.getBoundingClientRect(),b=layer.getBoundingClientRect();
  return Math.abs(a.x-b.x)<.5&&Math.abs(a.y-b.y)<.5&&Math.abs(a.width-b.width)<.5&&Math.abs(a.height-b.height)<.5;
 }));
}

for(const size of [{width:1120,height:760,scale:1},{width:960,height:640,scale:1.5}])test(`settings alignment and search interaction ${size.width}`,async({browser})=>{
 const context=await browser.newContext({viewport:size,deviceScaleFactor:size.scale,reducedMotion:'reduce'}),page=await context.newPage();
 await page.goto('/');await page.getByRole('button',{name:'设置',exact:true}).click();
 const dialog=page.getByRole('dialog',{name:'设置',exact:true});
 await expect(dialog.getByRole('combobox',{name:'录音设备'})).toBeEnabled();await page.evaluate(()=>document.fonts.ready);
 const rows=dialog.locator('[data-settings-row]');await expect(rows).toHaveCount(3);
 const geometry=await rows.evaluateAll(nodes=>nodes.map(node=>{const children=Array.from(node.children).slice(0,2).map(el=>{const r=el.getBoundingClientRect();return {x:r.x,right:r.right,y:r.y,bottom:r.bottom};});return {copy:children[0]!,control:children[1]!};}));
 for(const row of geometry){expect(Math.abs(row.copy.x-geometry[0]!.copy.x)).toBeLessThan(1);expect(Math.abs(row.control.right-geometry[0]!.control.right)).toBeLessThan(1);expect(row.control.x).toBeGreaterThan(row.copy.x);}
 await settled(page);await page.screenshot({path:`.local/search-settings/device-${size.width}.png`});
 // Exercise the content breakpoint independently of the supported desktop minimum.
 await dialog.evaluate(el=>{el.style.width='740px';});
 await expect.poll(()=>rows.evaluateAll(nodes=>nodes.every(node=>{const copy=node.children[0]!.getBoundingClientRect(),control=node.children[1]!.getBoundingClientRect();return control.y>=copy.bottom&&control.right<=node.getBoundingClientRect().right+.5;}))).toBe(true);
 await dialog.evaluate(el=>{el.style.removeProperty('width');});
 await dialog.getByRole('button',{name:'更改快捷键'}).click();await expect(dialog.getByText('按下快捷键…')).toBeVisible();await page.keyboard.press('Escape');
 await dialog.getByRole('button',{name:'模型',exact:true}).click();await expect(dialog.getByLabel('API 密钥')).toBeVisible();
 await settled(page);await page.screenshot({path:`.local/search-settings/models-speech-${size.width}.png`});
 await dialog.getByRole('tab',{name:'文字处理'}).click();await expect(dialog.getByRole('button',{name:'保存',exact:true})).toBeVisible();
 await settled(page);await page.screenshot({path:`.local/search-settings/models-text-${size.width}.png`});
 await dialog.getByRole('button',{name:'外观',exact:true}).click();await settled(page);await page.screenshot({path:`.local/search-settings/appearance-${size.width}.png`});
 await dialog.getByRole('button',{name:'关于',exact:true}).click();await settled(page);await page.screenshot({path:`.local/search-settings/about-${size.width}.png`});
 expect(await dialog.evaluate(el=>el.scrollWidth<=el.clientWidth)).toBe(true);
 await dialog.getByRole('button',{name:'关闭',exact:true}).click();
 for(const [route,trigger,label] of [['history','搜索识别文字','搜索识别文字'],['dictionary','搜索词语','搜索词条']]){
  await page.goto('/#/'+route);const search=page.getByRole('button',{name:trigger!});const slot=page.locator('[data-expandable-search]');
  await search.focus();await page.keyboard.press('Enter');const field=page.getByRole('textbox',{name:label!});await expect(field).toBeFocused();
  await expect(slot).toHaveCSS('width','216px');await expect(slot).toHaveCSS('height','36px');
  await field.fill('测试');await page.getByRole('heading',{level:1}).click();await expect(field).toHaveValue('测试');
  await expect(page.getByText(route==='history'?'没有匹配的记录':'没有匹配的词条',{exact:true})).toBeVisible();await settled(page);await page.screenshot({path:`.local/search-settings/${route}-search-${size.width}.png`});
  await field.focus();await page.keyboard.press('Escape');await expect(search).toBeFocused();
  await page.emulateMedia({reducedMotion:'reduce'});await search.click();
  expect(await field.evaluate(el=>getComputedStyle(el.parentElement!.parentElement!,'::before').animationName)).toBe('none');
  await page.keyboard.press('Escape');expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
 }
 await context.close();
});
