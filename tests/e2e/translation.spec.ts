import {test,expect} from '@playwright/test';
for(const theme of ['light','graphite'])test(`language rows reveal delete on hover and focus without moving layout: ${theme}`,async({page})=>{
 await page.emulateMedia({reducedMotion:'reduce'});await page.setViewportSize({width:960,height:640});await page.goto('/');
 await page.evaluate(async theme=>{const {applyTheme}=await import('../../src/design/themes');applyTheme(theme);},theme);
 await page.getByRole('button',{name:'设置',exact:true}).click();const dialog=page.getByRole('dialog',{name:'设置'});
 await dialog.getByRole('button',{name:'语言与翻译',exact:true}).click();
 const row=dialog.locator('[data-translation-language="ja"]'),remove=row.getByRole('button',{name:'移除日语'}),tools=remove.locator('..');
 const other=dialog.locator('[data-translation-language="en-US"]');
 await page.mouse.move(0,0);await expect(tools).toHaveCSS('opacity','0');
 const before=await row.boundingBox(),otherBefore=await other.boundingBox();await row.hover();await expect(tools).toHaveCSS('opacity','1');
 expect(await row.evaluate(n=>getComputedStyle(n,'::before').opacity)).toBe('1');expect(await row.boundingBox()).toEqual(before);expect(await other.boundingBox()).toEqual(otherBefore);
 await page.mouse.move(0,0);await expect(tools).toHaveCSS('opacity','0');await remove.focus();await expect(tools).toHaveCSS('opacity','1');
 await page.screenshot({path:`.local/translation-row-hover-${theme}.png`});
 await page.keyboard.press('Enter');await expect(row).toHaveCount(0);
 for(const label of ['韩语','法语','中文（简体）']){const button=dialog.getByRole('button',{name:`移除${label}`});await button.locator('xpath=ancestor::*[@data-translation-language]').hover();await button.click();}
 await other.hover();await expect(other.getByRole('button',{name:'移除英语（美国）'})).toBeDisabled();
});
test('touch language list exposes delete without hover',async({browser})=>{
 const context=await browser.newContext({hasTouch:true,viewport:{width:960,height:640}}),page=await context.newPage();
 await page.goto('/');await page.getByRole('button',{name:'设置',exact:true}).click();const dialog=page.getByRole('dialog',{name:'设置'});
 await dialog.getByRole('button',{name:'语言与翻译',exact:true}).click();
 const row=dialog.locator('[data-translation-language="ja"]');await expect(row.getByRole('button',{name:'移除日语'}).locator('..')).toHaveCSS('opacity','1');
 await row.getByRole('button',{name:'移除日语'}).tap();await expect(row).toHaveCount(0);await context.close();
});
test('all supported languages remain selectable in a scrolling capsule menu',async({page})=>{
 await page.setViewportSize({width:280,height:300});
 await page.goto('/tests/fixtures/voice-recovery.html?surface=translation&languages=all');
 const trigger=page.getByRole('button',{name:'翻译目标语言'});
 await trigger.click();await expect(page.getByRole('menuitemradio')).toHaveCount(22);
 const menu=page.getByRole('menu'),rect=(await menu.boundingBox())!;
 expect(rect.y).toBeGreaterThanOrEqual(0);expect(rect.y+rect.height).toBeLessThanOrEqual(300);
 expect(await menu.evaluate(n=>n.scrollHeight>n.clientHeight)).toBe(true);
 const last=page.getByRole('menuitemradio',{name:'马来语',exact:true});await last.scrollIntoViewIfNeeded();await last.click();
 await expect(trigger).toContainText('马来语');await expect(menu).toHaveCount(0);
 await trigger.click();await expect(last).toHaveAttribute('aria-checked','true');
 await page.mouse.click(5,5);await expect(menu).toHaveCount(0);
});
for(const theme of ['light','graphite'])for(const scale of [1,1.5])test(`translation target menu ${theme} DPI ${scale}`,async({browser})=>{
 const context=await browser.newContext({viewport:{width:280,height:300},deviceScaleFactor:scale,reducedMotion:'reduce'}),page=await context.newPage();
 await page.goto('/tests/fixtures/voice-recovery.html?surface=translation');
 await page.evaluate(async theme=>{const {applyTheme}=await import('../../src/design/themes');applyTheme(theme);},theme);
 const trigger=page.getByRole('button',{name:'翻译目标语言'});
 await expect(trigger).toContainText('英语（美国）');await expect(page.getByText('This raw transcript must not be rendered')).toHaveCount(0);
 // Compare layout allocation independently of capsule entrance motion.
 const layout=()=>trigger.evaluate(node=>{const n=node as HTMLElement;return [n.offsetLeft,n.offsetTop,n.offsetWidth,n.offsetHeight];});
 const before=await layout();await trigger.click();
 const option=page.getByRole('menuitemradio',{name:'日语'});await expect(option).toBeVisible();await option.hover();
 for(const item of await page.getByRole('menuitemradio').all()){const r=(await item.boundingBox())!;expect(r.x).toBeGreaterThanOrEqual(0);expect(r.y).toBeGreaterThanOrEqual(0);expect(r.x+r.width).toBeLessThanOrEqual(280);expect(r.y+r.height).toBeLessThanOrEqual(300);}
 expect(await layout()).toEqual(before);
 await page.screenshot({path:`.local/translation-menu-${theme}-${scale}.png`});
 await option.click();await expect(trigger).toContainText('日语');await expect(option).toHaveCount(0);
 const selectedLayout=await layout();
 await trigger.click();
 const current=page.getByRole('menuitemradio',{name:'日语',exact:true});await expect(current).toHaveAttribute('aria-checked','true');await current.click();await expect(page.getByRole('menu')).toHaveCount(0);
 for(let i=0;i<6;i++){await trigger.click();await expect(page.getByRole('menu')).toBeVisible();expect(await layout()).toEqual(selectedLayout);await page.mouse.click(5,5);await expect(page.getByRole('menu')).toHaveCount(0);expect(await layout()).toEqual(selectedLayout);}
 await page.getByRole('button',{name:'结束录音'}).click();await expect(trigger).toBeDisabled();
 await context.close();
});
test('translation settings and shortcut feedback fit minimum window',async({page})=>{
 await page.setViewportSize({width:960,height:640});await page.goto('/');await page.getByRole('button',{name:'设置',exact:true}).click();
 const dialog=page.getByRole('dialog',{name:'设置'});await expect(dialog.getByRole('button',{name:'更改翻译快捷键'})).toContainText('右 Alt');
 await dialog.getByRole('button',{name:'语言与翻译',exact:true}).click();await expect(dialog.getByRole('button',{name:'移除日语'})).toBeEnabled();
 for(const label of ['英语（美国）','日语','韩语','法语','中文（简体）'])await expect(dialog.getByRole('button',{name:`移除${label}`})).toBeVisible();
 await expect(dialog.getByRole('combobox',{name:'添加翻译语言'})).toBeEnabled();
 await expect(dialog.getByRole('button',{name:/上移|下移/})).toHaveCount(0);
 await page.screenshot({path:'.local/translation-settings-minimum.png'});
 expect(await dialog.evaluate(node=>node.scrollWidth<=node.clientWidth)).toBe(true);
});

test('reserved shortcut stays local to its key field and preserves the effective binding',async({page})=>{
 await page.goto('/');await page.getByRole('button',{name:'设置',exact:true}).click();
 const dialog=page.getByRole('dialog',{name:'设置'});
 for(const label of ['更改快捷键','更改翻译快捷键']){
  const key=dialog.getByRole('button',{name:label,exact:true});await key.click();await page.keyboard.press('AltLeft');
  await expect(key).toHaveAttribute('aria-invalid','true');await expect(key).toContainText('左 Alt');
  await expect(dialog.locator('#'+await key.getAttribute('aria-describedby'))).toContainText('此快捷键已保留供系统使用');
  await expect(dialog.getByText('已启用：右 Alt',{exact:true})).toBeVisible();
  await dialog.getByRole('button',{name:'重新读取'}).click();await expect(key).toHaveAttribute('aria-invalid','false');
 }
});
