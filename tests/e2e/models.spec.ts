import {test,expect,type Locator,type Page} from '@playwright/test';
async function open(page:Page){await page.goto('/');await page.getByRole('button',{name:'配置模型',exact:true}).click();const dialog=page.getByRole('dialog');await expect(dialog.getByLabel('API 密钥')).toBeVisible();return dialog;}
async function hitVisible(locator:Locator){
 await expect.poll(()=>locator.evaluate(el=>{const r=el.getBoundingClientRect(),x=r.left+r.width/2,y=r.top+r.height/2;const top=document.elementFromPoint(x,y);return r.width>0&&r.height>0&&x>=0&&y>=0&&x<innerWidth&&y<innerHeight&&!!top&&(el===top||el.contains(top));})).toBe(true);
}
test('task-based model settings save, select, edit and discard with focus restoration',async({page})=>{
 const dialog=await open(page);await expect(dialog.getByRole('tab',{name:'语音识别'})).toHaveAttribute('aria-selected','true');
 await dialog.getByRole('tab',{name:'文字处理'}).click();
 await dialog.getByLabel('服务地址（URL）',{exact:true}).fill('https://example.invalid/v1');
 await dialog.getByLabel('API 密钥',{exact:true}).fill('fixture-key');
 await dialog.getByLabel('模型',{exact:true}).fill('demo-text-model');
 await dialog.getByRole('button',{name:'测试连接'}).click();await expect(dialog.getByText('模拟连接成功',{exact:true})).toBeVisible();
 await dialog.getByRole('button',{name:'保存',exact:true}).click();await expect(dialog.getByText('已保存。')).toBeVisible();
 await expect(dialog.getByText('当前使用',{exact:true})).toBeVisible();
 await page.keyboard.press('Escape');await expect(dialog).toHaveCount(0);await expect(page.getByRole('button',{name:'配置模型',exact:true})).toBeFocused();
 await page.getByRole('button',{name:'配置模型',exact:true}).click();await dialog.getByRole('tab',{name:'文字处理'}).click();
 await dialog.getByRole('button',{name:'编辑',exact:true}).click();await expect(dialog.getByLabel('API 密钥',{exact:true})).toHaveValue('');
 await dialog.getByLabel('模型',{exact:true}).fill('updated-model');
 await dialog.getByRole('button',{name:'关闭设置',exact:true}).click();
 const confirm=page.getByRole('alertdialog');await expect(confirm).toBeVisible();await hitVisible(confirm.getByRole('button',{name:'继续填写'}));
 await confirm.getByRole('button',{name:'继续填写'}).click();await expect(dialog.getByLabel('模型',{exact:true})).toHaveValue('updated-model');
 await page.keyboard.press('Escape');await confirm.getByRole('button',{name:'放弃修改'}).click();await expect(dialog).toHaveCount(0);
 await expect(page.getByRole('button',{name:'配置模型',exact:true})).toBeFocused();
});
test('necessary fields and fixed actions remain clickable at both window sizes',async({page})=>{
 for(const size of [{width:1120,height:760},{width:960,height:640}]){
  await page.setViewportSize(size);const dialog=await open(page);
  await hitVisible(dialog.getByLabel('API 密钥'));await hitVisible(dialog.getByRole('button',{name:'保存',exact:true}));await hitVisible(dialog.getByRole('button',{name:'关闭设置',exact:true}));
  await expect(dialog.getByLabel('名称')).toHaveCount(0);await expect(dialog.getByLabel('服务地址（URL）')).toHaveCount(0);
  await page.screenshot({path:'tests/visual/settings-speech-'+size.width+'.png'});
  await dialog.getByRole('tab',{name:'文字处理'}).click();await hitVisible(dialog.getByLabel('API 密钥'));await hitVisible(dialog.getByLabel('模型',{exact:true}));await hitVisible(dialog.getByRole('button',{name:'保存',exact:true}));
  await page.screenshot({path:'tests/visual/settings-text-'+size.width+'.png'});
  await dialog.getByRole('button',{name:'更多设置'}).click();
  await dialog.getByTestId('model-scroll').evaluate(el=>{el.scrollTop=el.scrollHeight;});
  await hitVisible(dialog.getByRole('button',{name:'保存',exact:true}));await hitVisible(dialog.getByRole('button',{name:'关闭设置',exact:true}));
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  const green=await dialog.locator('*').evaluateAll(nodes=>nodes.filter(n=>Array.from(n.childNodes).some(c=>c.nodeType===3&&c.textContent?.trim())&&getComputedStyle(n).color==='rgb(117, 251, 145)').length);expect(green).toBe(0);
  await page.keyboard.press('Escape');await expect(dialog).toHaveCount(0);
 }
});
test('close confirms visible changes from the bottom and cancels category changes safely',async({page})=>{
 const dialog=await open(page);await dialog.getByRole('button',{name:'更多设置'}).click();await dialog.getByLabel('名称').fill('我的语音');
 await dialog.getByTestId('model-scroll').evaluate(el=>{el.scrollTop=el.scrollHeight;});
 await dialog.getByRole('tab',{name:'文字处理'}).click();const confirm=page.getByRole('alertdialog');await hitVisible(confirm.getByRole('button',{name:'继续填写'}));
 expect(await page.locator('[role="dialog"] button[aria-label="关闭设置"]').evaluate(el=>{const r=el.getBoundingClientRect();return !el.contains(document.elementFromPoint(r.left+r.width/2,r.top+r.height/2));})).toBe(true);
 await confirm.getByRole('button',{name:'继续填写'}).click();await expect(dialog.getByRole('tab',{name:'语音识别'})).toHaveAttribute('aria-selected','true');
 await page.keyboard.press('Escape');await confirm.getByRole('button',{name:'放弃修改'}).click();await expect(dialog).toHaveCount(0);
});
test('empty keys focus the error and pristine close works through button and backdrop',async({page})=>{
 let dialog=await open(page);await dialog.getByRole('button',{name:'关闭设置',exact:true}).click();await expect(dialog).toHaveCount(0);
 dialog=await open(page);await page.mouse.click(8,8);await expect(dialog).toHaveCount(0);
 dialog=await open(page);await dialog.getByRole('button',{name:'保存',exact:true}).click();await expect(dialog.getByLabel('API 密钥')).toBeFocused();await expect(dialog.getByLabel('API 密钥')).toHaveAttribute('aria-invalid','true');
 await hitVisible(dialog.getByRole('button',{name:'关闭设置',exact:true}));await dialog.getByRole('button',{name:'关闭设置',exact:true}).click();await expect(dialog).toHaveCount(0);
});
test('long model summaries, menus and delete confirmation stay within a small window',async({page})=>{
 await page.setViewportSize({width:960,height:640});const dialog=await open(page);
 await dialog.getByRole('combobox',{name:'模型',exact:true}).click();
 const option=page.getByRole('option',{name:'Qwen Audio 3.0 · 实时识别',exact:true});await hitVisible(option);await page.screenshot({path:'tests/visual/settings-model-menu-960.png'});await page.keyboard.press('Escape');
 await dialog.getByRole('tab',{name:'文字处理'}).click();
 await dialog.getByLabel('服务地址（URL）',{exact:true}).fill('https://example.invalid/v1');await dialog.getByLabel('API 密钥',{exact:true}).fill('fixture-key');
 await dialog.getByLabel('模型',{exact:true}).fill('model-'+ 'abcdefghij'.repeat(23));await dialog.getByRole('button',{name:'更多设置'}).click();
 const name='这是一份需要完整保留名称的个人模型'.repeat(3);await dialog.getByLabel('名称',{exact:true}).fill(name);
 await dialog.getByRole('button',{name:'保存',exact:true}).click();await expect(dialog.getByText('当前使用',{exact:true})).toBeVisible();
 expect(await dialog.evaluate(el=>el.scrollWidth<=el.clientWidth)).toBe(true);await hitVisible(dialog.getByRole('button',{name:'关闭设置',exact:true}));
 await dialog.getByRole('button',{name:'删除',exact:true}).click();const confirm=page.getByRole('alertdialog');await hitVisible(confirm.getByRole('button',{name:'确认删除'}));
 expect(await confirm.evaluate(el=>el.scrollWidth<=el.clientWidth)).toBe(true);await page.keyboard.press('Escape');await expect(confirm).toHaveCount(0);
});


test('model category tabs are lightweight, fixed and keyboard accessible',async({page})=>{
 const dialog=await open(page), speech=dialog.getByRole('tab',{name:'语音识别'}), text=dialog.getByRole('tab',{name:'文字处理'});
 const before={speech:await speech.boundingBox(),text:await text.boundingBox()};
 await speech.focus();await page.keyboard.press('ArrowRight');await expect(text).toBeFocused();await expect(speech).toHaveAttribute('aria-selected','true');await page.keyboard.press('Enter');await expect(text).toHaveAttribute('aria-selected','true');
 expect(await speech.boundingBox()).toEqual(before.speech);expect(await text.boundingBox()).toEqual(before.text);
 expect(await text.evaluate(el=>getComputedStyle(el).backgroundColor)).toBe('rgba(0, 0, 0, 0)');
 await hitVisible(dialog.getByLabel('服务地址（URL）'));await expect(dialog.getByLabel('模型',{exact:true})).toHaveValue('');
 await page.keyboard.press('Escape');await expect(dialog).toHaveCount(0);
});
