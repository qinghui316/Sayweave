import {test,expect} from '@playwright/test';
test('submitted input exposes a closeable notice and recovery inside its canvas',async({page})=>{
 await page.setViewportSize({width:384,height:240});
 await page.goto('/tests/fixtures/voice-recovery.html');
 const recovery=page.getByRole('button',{name:'重试恢复剪贴板'});
 await recovery.click();await expect(recovery).toBeEnabled();
 await expect(page.getByRole('button',{name:'复制',exact:true})).toHaveCount(0);
 for(const button of [recovery,page.getByRole('button',{name:'关闭',exact:true})]){
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
 await expect(page.getByText('识别中',{exact:true})).toBeVisible();
 await expect(page.getByLabel('实时字幕')).toHaveCount(0);await expect(page.getByRole('button',{name:'复制',exact:true})).toHaveCount(0);
 await page.setViewportSize({width:120,height:112});await page.goto('/tests/fixtures/voice-recovery.html?presentation=completed');
 await expect(page.getByText('已填入',{exact:true})).toBeVisible();
 const rect=await page.getByText('已填入',{exact:true}).boundingBox();expect(rect!.y+rect!.height).toBeLessThanOrEqual(112);
});

test('device and shortcut controls fit both window sizes with shared styles and keyboard operation',async({page})=>{
 await page.goto('/');await page.getByRole('button',{name:'设置',exact:true}).click();
 const dialog=page.getByRole('dialog',{name:'设置'});
 for(const [width,height] of [[1120,760],[960,640]] as const){
  await page.setViewportSize({width,height});
  const device=dialog.getByRole('combobox',{name:'录音设备'});
  await expect(device).toBeVisible();await device.click();await expect(page.getByRole('option',{name:'系统默认麦克风',exact:true})).toBeVisible();await page.keyboard.press('Escape');
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
