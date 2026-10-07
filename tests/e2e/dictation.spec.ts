import {test,expect} from '@playwright/test';
for(const reducedMotion of ['no-preference','reduce'] as const)test(`compact overlay keeps its anchor and visual scale across animated phases: ${reducedMotion}`,async({page})=>{
 await page.emulateMedia({reducedMotion});await page.setViewportSize({width:160,height:88});
 await page.goto('/tests/fixtures/voice-recovery.html?presentation=preparing');
 const shell=page.locator('[data-phase]');
 await page.evaluate(()=>document.fonts.ready);
 await expect.poll(async()=>{const rect=await shell.boundingBox();return rect&&[rect.x+rect.width/2,rect.y+rect.height,rect.width,rect.height];}).toEqual([80,76,72,34]);
 for(const [kind,width] of [['preparing',72],['recording',136],['recognizing',72],['completed',34]] as const){
  const samples=await page.evaluate(async(kind)=>{
   window.dispatchEvent(new CustomEvent('fixture:presentation',{detail:kind}));
   const frames:{center:number;bottom:number;height:number;buttonsInside:boolean}[]=[];
   const start=performance.now();
   await new Promise<void>(resolve=>{const frame=()=>{const shell=document.querySelector('[data-phase]')!,rect=shell.getBoundingClientRect();const buttonsInside=Array.from(shell.querySelectorAll('button')).every(button=>{const b=button.getBoundingClientRect();return b.left>=rect.left&&b.right<=rect.right&&b.top>=rect.top&&b.bottom<=rect.bottom&&button.contains(document.elementFromPoint(b.x+b.width/2,b.y+b.height/2));});frames.push({center:rect.x+rect.width/2,bottom:rect.bottom,height:rect.height,buttonsInside});if(performance.now()-start<400)requestAnimationFrame(frame);else resolve();};requestAnimationFrame(frame);});
   return frames;
  },kind);
  for(const frame of samples){expect(frame.center).toBeCloseTo(80,1);expect(frame.bottom).toBeCloseTo(76,1);expect(frame.height).toBe(34);expect(frame.buttonsInside).toBe(true);}
  expect((await shell.boundingBox())!.width).toBe(width);
  for(const button of await shell.getByRole('button').all()){const rect=(await button.boundingBox())!;expect(rect.width).toBeGreaterThanOrEqual(24);expect(rect.height).toBeGreaterThanOrEqual(24);expect(rect.x).toBeGreaterThanOrEqual(0);expect(rect.x+rect.width).toBeLessThanOrEqual(160);}
  await page.screenshot({path:`tests/visual/compact-overlay-${kind}-${reducedMotion}.png`});
 }
});
test('interrupted input exposes only a compact closeable notice',async({page})=>{
 await page.setViewportSize({width:384,height:240});
 await page.goto('/tests/fixtures/voice-recovery.html');
 await expect(page.getByRole('status')).toHaveText('请确认输入结果。');
 await expect(page.getByRole('button',{name:'重试恢复剪贴板'})).toHaveCount(0);
 await expect(page.getByRole('button',{name:'复制',exact:true})).toHaveCount(0);
 for(const button of [page.getByRole('button',{name:'关闭',exact:true})]){
  const rect=await button.boundingBox();expect(rect).not.toBeNull();
  expect(rect!.y).toBeGreaterThanOrEqual(0);expect(rect!.y+rect!.height).toBeLessThanOrEqual(240);
  expect(rect!.x).toBeGreaterThanOrEqual(0);expect(rect!.x+rect!.width).toBeLessThanOrEqual(384);
 }
 await page.getByRole('button',{name:'关闭',exact:true}).click();await expect(page.getByRole('status')).toHaveText('已关闭');
});
test('no-target results retain full text and close after failed copy without clipping',async({page})=>{
 await page.setViewportSize({width:384,height:232});
 await page.goto('/tests/fixtures/voice-recovery.html?presentation=result');
 await page.getByRole('button',{name:'复制',exact:true}).click();
 await expect(page.getByRole('button',{name:'复制失败，重试'})).toBeVisible();
 for(const button of [page.getByRole('button',{name:'复制失败，重试'}),page.getByRole('button',{name:'关闭',exact:true})]){
  const rect=await button.boundingBox();expect(rect!.y+rect!.height).toBeLessThanOrEqual(232);
 }
 await expect(page.getByRole('region',{name:'口述结果'})).toContainText('完整文字供复制。'.repeat(40));
 await page.getByRole('button',{name:'关闭',exact:true}).click();await expect(page.getByRole('status')).toHaveText('已关闭');
});
test('normal finish shows recognition feedback and hides terminal captions',async({page})=>{
 await page.setViewportSize({width:384,height:220});
 await page.goto('/tests/fixtures/voice-recovery.html?presentation=recognizing');
 await expect(page.getByRole('group',{name:'识别中',exact:true})).toBeVisible();await expect(page.getByText('识别中',{exact:true})).toHaveCount(0);
 await expect(page.getByLabel('实时字幕')).toHaveCount(0);await expect(page.getByRole('button',{name:'复制',exact:true})).toHaveCount(0);
 await page.setViewportSize({width:120,height:112});await page.goto('/tests/fixtures/voice-recovery.html?presentation=completed');
 await expect(page.getByRole('group',{name:'口述完成',exact:true})).toBeVisible();await expect(page.getByText('口述完成',{exact:true})).toHaveCount(0);
 const rect=await page.getByRole('group',{name:'口述完成',exact:true}).boundingBox();expect(rect!.y+rect!.height).toBeLessThanOrEqual(112);
});

test('device and shortcut controls fit both window sizes with shared styles and keyboard operation',async({page})=>{
 await page.goto('/');await page.getByRole('button',{name:'设置',exact:true}).click();
 const dialog=page.getByRole('dialog',{name:'设置'});
 for(const [width,height] of [[1120,760],[960,640]] as const){
  await page.setViewportSize({width,height});
  const device=dialog.getByRole('combobox',{name:'录音设备'});
  await expect(device).toBeVisible();await device.click();await expect(page.getByRole('option',{name:'系统默认麦克风',exact:true})).toBeFocused();await page.keyboard.press('Escape');await expect(page.getByRole('listbox')).toHaveCount(0);await expect(dialog).toBeVisible();
  const keyField=dialog.getByRole('button',{name:'更改快捷键'});await keyField.focus();await page.keyboard.press('Enter');
  await expect(keyField).toHaveAttribute('aria-pressed','true');await expect(keyField).toHaveText('按下快捷键…');
  await expect(dialog.getByRole('button',{name:'取消录入'})).toBeVisible();await expect(dialog.getByRole('button',{name:'保存快捷键'})).toBeDisabled();
  await dialog.getByRole('button',{name:'取消录入'}).click();await expect(keyField).toHaveAttribute('aria-pressed','false');await expect(keyField).toHaveText('右 Alt');
  await keyField.click();await expect(keyField).toHaveAttribute('aria-pressed','true');await dialog.getByRole('heading',{name:'麦克风',exact:true}).click();await expect(keyField).toHaveAttribute('aria-pressed','false');
  const chosen=width===1120?'按住':'点按';await dialog.getByRole('tab',{name:chosen,exact:true}).click();await expect(dialog.getByRole('tab',{name:chosen,exact:true})).toHaveAttribute('aria-selected','true');
  await expect(dialog.getByText(chosen==='按住'?'按下开始，松开结束。':'按一下开始，再按一下结束。')).toBeVisible();
  for(const control of [device,dialog.getByRole('button',{name:'更改快捷键'}),dialog.getByRole('tab',{name:'点按',exact:true}),dialog.getByRole('tab',{name:'按住',exact:true}),dialog.getByRole('button',{name:'保存快捷键'})]){
   const unclipped=await control.evaluate(node=>{const box=node.getBoundingClientRect();for(let parent=node.parentElement;parent;parent=parent.parentElement){const css=getComputedStyle(parent);if(['auto','scroll','hidden'].includes(css.overflowY)){const clip=parent.getBoundingClientRect();if(box.top<clip.top||box.bottom>clip.bottom)return false;}}return true;});expect(unclipped).toBe(true);
   const rect=await control.boundingBox();expect(rect).not.toBeNull();expect(rect!.height).toBeGreaterThanOrEqual(36);expect(rect!.y).toBeGreaterThanOrEqual(0);expect(rect!.y+rect!.height).toBeLessThan(height);expect(rect!.x+rect!.width).toBeLessThan(width);
  }
  await expect(dialog.getByText('全局快捷键',{exact:true})).toHaveCount(0);await expect(dialog.getByText('未配置',{exact:true})).toHaveCount(0);
  await page.screenshot({path:`.local/dictation-settings-${width}.png`});
  await dialog.getByRole('button',{name:'保存快捷键'}).click();await expect(dialog.getByRole('button',{name:'保存快捷键'})).toBeDisabled();
 }
});
