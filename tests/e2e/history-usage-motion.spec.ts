import {test,expect} from '@playwright/test';

for(const size of [{width:1120,height:760,scale:1},{width:960,height:640,scale:1.5}])test(`history positions and restrained usage tilt ${size.width}`,async({browser})=>{
 const context=await browser.newContext({viewport:size,deviceScaleFactor:size.scale,recordVideo:{dir:'.local/release-0113/ui/video',size:{width:size.width,height:size.height}}});
 const page=await context.newPage();await page.goto('/#/history');
 const slot=page.locator('[data-expandable-search]'),search=page.getByRole('button',{name:'搜索识别文字'}),refresh=page.getByRole('button',{name:'刷新历史',exact:true});
 await expect(slot).toHaveAttribute('data-anchor','left');
 const initial=await refresh.boundingBox(),closed=await search.boundingBox();expect(initial!.x).toBeGreaterThan(closed!.x+216);
 await page.evaluate(()=>document.fonts.ready);await page.screenshot({path:`.local/release-0113/ui/history-closed-${size.width}.png`});
 await search.click();const input=page.getByRole('textbox',{name:'搜索识别文字'});await expect(input).toBeFocused();
 await expect.poll(async()=>Math.abs((await refresh.boundingBox())!.x-initial!.x)).toBeLessThan(.5);
  const open=await input.boundingBox();expect(open!.x).toBeLessThanOrEqual(closed!.x+36);
 await input.fill('测试');await input.dispatchEvent('compositionstart');await input.dispatchEvent('keydown',{key:'Escape',isComposing:true});await expect(input).toHaveValue('测试');await input.dispatchEvent('compositionend');
 await page.getByRole('heading',{level:1}).click();await expect(input).toHaveValue('测试');
 await input.focus();await page.keyboard.press('Escape');await expect(search).toBeFocused();await refresh.click();
 await search.click();await expect(input).toBeFocused();await page.screenshot({path:`.local/release-0113/ui/history-open-${size.width}.png`});
 await page.goto('/#/dictionary');await expect(page.locator('[data-expandable-search]')).toHaveAttribute('data-anchor','right');
 await page.goto('/');const area=page.locator('section[aria-labelledby="usage-title"]'),face=area.locator(':scope > div').nth(1),back=area.locator(':scope > div').first();
 const rect=(await area.boundingBox())!,backBefore=await back.boundingBox(),faceBefore=(await face.boundingBox())!;const body=await face.innerText();
 for(const [x,y] of [[.5,.5],[.05,.05],[.95,.95],[.05,.95]]){
  await page.mouse.move(rect.x+rect.width*x!,rect.y+rect.height*y!,{steps:8});await expect(face).toHaveAttribute('data-tilting','true');
  const angles=await face.evaluate(el=>['--tilt-x','--tilt-y'].map(name=>parseFloat((el as HTMLElement).style.getPropertyValue(name))));
  expect(angles.every(angle=>Math.abs(angle)<=3)).toBe(true);expect(await back.boundingBox()).toEqual(backBefore);
 }
 await page.screenshot({path:`.local/release-0113/ui/usage-tilt-${size.width}.png`});expect(await face.innerText()).toBe(body);
 await page.mouse.move(5,5);await expect(face).not.toHaveAttribute('data-tilting');await expect.poll(async()=>{const box=(await face.boundingBox())!;return Math.max(Math.abs(box.x-faceBefore.x),Math.abs(box.y-faceBefore.y),Math.abs(box.width-faceBefore.width),Math.abs(box.height-faceBefore.height));}).toBeLessThan(.1);
 const info=area.getByRole('button');await info.focus();await expect(info).toBeFocused();
 await page.emulateMedia({reducedMotion:'reduce'});await page.mouse.move(rect.x+10,rect.y+10);await expect(face).not.toHaveAttribute('data-tilting');await expect(face).toHaveCSS('transform','none');
 await page.screenshot({path:`.local/release-0113/ui/usage-static-${size.width}.png`});
 const video=page.video();await context.close();await video?.saveAs(`.local/release-0113/ui/motion-${size.width}.webm`);
});
