import {test,expect} from '@playwright/test';
test.use({video:'on'});
test('shared controls retain geometry, keyboard semantics and latest controlled selection',async({page},info)=>{
 await page.setViewportSize({width:960,height:640});await page.goto('/#/dev/components');
 const select=page.getByRole('combobox',{name:'选项',exact:true});await select.scrollIntoViewIfNeeded();
 await select.click();const options=page.getByRole('option');await expect(options).toHaveCount(3);
 const layer=page.locator('[data-selection-highlight]');await expect(layer).toHaveCSS('opacity','1');await expect(layer).toHaveCSS('transition-property','transform, opacity');
 const chosen=options.first();await chosen.press('ArrowDown');
 await expect(options.nth(1)).toBeFocused();await expect(chosen.locator('[class*="selectionMark"]')).toBeVisible();
 await expect.poll(async()=>{const a=await layer.boundingBox(),b=await options.nth(1).boundingBox();return !!a&&!!b&&Math.abs(a.y-b.y)<1&&Math.abs(a.height-b.height)<1;}).toBe(true);
 await page.screenshot({path:info.outputPath('select.png')});await page.keyboard.press('Escape');await expect(select).toBeFocused();
 const radio=page.getByRole('radiogroup',{name:'口述方式示例'}),toggle=radio.getByRole('radio',{name:'点按'}),hold=radio.getByRole('radio',{name:'按住'});
 await toggle.scrollIntoViewIfNeeded();const box=await radio.boundingBox();await toggle.focus();await page.keyboard.press('ArrowRight');await expect(hold).toBeChecked();expect(await radio.boundingBox()).toEqual(box);
 for(let i=0;i<4;i++){await toggle.click({force:true});await hold.click({force:true});}await expect(hold).toBeChecked();
 const control=page.getByRole('switch',{name:'开关',exact:true});const switchBox=await control.boundingBox();await control.click();await expect(control).toBeChecked();expect(await control.boundingBox()).toEqual(switchBox);
 await control.press('Space');await expect(control).not.toBeChecked();
 await page.emulateMedia({reducedMotion:'reduce'});await hold.click();await toggle.click();await expect(toggle).toBeChecked();await select.click();await expect(layer).toHaveCSS('transition-duration','0s');await page.keyboard.press('Escape');
 await page.screenshot({path:info.outputPath('controls-reduced.png')});
});
test('waveform moves independent bars without changing its capsule layout',async({page},info)=>{
 await page.setViewportSize({width:1120,height:760});await page.goto('/');
 await page.getByRole('button',{name:'开始录音',exact:true}).click();const wave=page.locator('[data-live="true"]');await expect(wave).toBeVisible();
 const capsule=page.locator('[data-phase="recording"]');await expect(capsule).toHaveCSS('width','136px');await expect(capsule).toHaveCSS('height','34px');const bounds=await capsule.boundingBox();
 const values=()=>wave.locator('i').evaluateAll(nodes=>nodes.map(n=>getComputedStyle(n).transform));
 const first=await values();await expect.poll(values).not.toEqual(first);expect(new Set(await values()).size).toBeGreaterThan(5);expect(await capsule.boundingBox()).toEqual(bounds);
 await page.screenshot({path:info.outputPath('waveform.png')});
 await page.getByRole('button',{name:'结束录音',exact:true}).click();await expect(wave).toHaveCount(0);
 await page.setViewportSize({width:960,height:640});await expect(page.locator('main')).toBeVisible();
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
});


test('glide highlight follows a scrolled viewport and keeps the last browsed row',async({page})=>{
 await page.setViewportSize({width:960,height:640});await page.goto('/#/dev/components');
 await page.getByRole('combobox',{name:'滚动选项示例'}).click();
 const items=page.getByRole('option'),last=items.last(),layer=page.locator('[data-selection-highlight]');
 await items.first().press('End');await expect(last).toBeFocused();
 await expect.poll(async()=>{const a=await layer.boundingBox(),b=await last.boundingBox();return !!a&&!!b&&Math.abs(a.y-b.y)<1;}).toBe(true);
 await last.hover();const box=await last.boundingBox();await page.mouse.move(5,5);
 await expect.poll(async()=>{const a=await layer.boundingBox();return !!a&&!!box&&Math.abs(a.y-box.y)<1;}).toBe(true);
 await page.keyboard.press('Escape');
});
