import {test,expect,type Locator,type Page} from '@playwright/test';
async function open(page:Page){await page.goto('/');await page.getByRole('button',{name:'配置模型',exact:true}).click();const dialog=page.getByRole('dialog');await expect(dialog.getByLabel('API 密钥')).toBeVisible();return dialog;}
async function manualModel(dialog:Locator){const button=dialog.getByRole('button',{name:'手动填写',exact:true});if(await button.count())await button.click();}
async function modelAction(page:Page,root:Locator,label:string){await root.getByRole('button',{name:/^更多操作 /u}).first().click();await page.getByRole('menuitem',{name:label,exact:true}).click();}
async function hitVisible(locator:Locator){
 await expect.poll(()=>locator.evaluate(el=>{const r=el.getBoundingClientRect(),x=r.left+r.width/2,y=r.top+r.height/2;const top=document.elementFromPoint(x,y);return r.width>0&&r.height>0&&x>=0&&y>=0&&x<innerWidth&&y<innerHeight&&!!top&&(el===top||el.contains(top));})).toBe(true);
}
async function highlightAt(menu:Locator,item:Locator){
 const layer=menu.locator('[data-selection-highlight]');await expect(layer).toHaveCSS('opacity','1');
 await expect.poll(async()=>{const a=await layer.boundingBox(),b=await item.boundingBox();return !!a&&!!b&&Math.abs(a.x-b.x)<1&&Math.abs(a.y-b.y)<1&&Math.abs(a.width-b.width)<1&&Math.abs(a.height-b.height)<1;}).toBe(true);
}
test('local model controls remain usable in the compact settings window without changing the default on download',async({page})=>{
 await page.setViewportSize({width:960,height:640});
 const dialog=await open(page), local=dialog.getByRole('region',{name:'本地语音模型'});
 await local.getByRole('button',{name:'下载',exact:true}).click();
 const chooseLocal=local.getByRole('button',{name:'Nemotron 3.5 · 本地流式识别',exact:true});
 await expect(chooseLocal).toBeEnabled();
 await expect(chooseLocal).toHaveAttribute('aria-pressed','false');
 await modelAction(page,local,'计算方式');
 const device=page.getByRole('combobox',{name:'本地识别计算方式'});
 await device.click();await page.getByRole('option',{name:'仅 CPU',exact:true}).click();
 await expect(device).toHaveText('仅 CPU');
 await page.getByRole('button',{name:'完成',exact:true}).click();
 await chooseLocal.click();
 await expect(chooseLocal).toHaveAttribute('aria-pressed','true');
 await modelAction(page,local,'删除模型');
 const confirm=page.getByRole('alertdialog');await hitVisible(confirm.getByRole('button',{name:'确认删除',exact:true}));
 await confirm.getByRole('button',{name:'确认删除',exact:true}).click();
 await expect(confirm).toHaveCount(0);await expect(local.getByRole('button',{name:'下载',exact:true})).toBeVisible();
 await expect(chooseLocal).toHaveAttribute('aria-pressed','false');
});
test('task-based model settings save, select, edit and discard with focus restoration',async({page})=>{
 const dialog=await open(page);await expect(dialog.getByRole('tab',{name:'语音识别'})).toHaveAttribute('aria-selected','true');
 await dialog.getByRole('tab',{name:'文字处理'}).click();
 await dialog.getByLabel('服务地址（URL）',{exact:true}).fill('https://example.invalid/v1');
 await dialog.getByLabel('API 密钥',{exact:true}).fill('fixture-key');
 await manualModel(dialog);await dialog.getByLabel('模型',{exact:true}).fill('demo-text-model');
 await dialog.getByRole('button',{name:'测试连接'}).click();await expect(dialog.getByText('模拟连接成功',{exact:true})).toBeVisible();
 await dialog.getByRole('button',{name:'保存',exact:true}).click();await expect(dialog.getByText('已保存。')).toBeVisible();
 await expect(dialog.locator('[data-model-card][data-selected=true]')).toBeVisible();
 await page.keyboard.press('Escape');await expect(dialog).toHaveCount(0);await expect(page.getByRole('button',{name:'配置模型',exact:true})).toBeFocused();
 await page.getByRole('button',{name:'配置模型',exact:true}).click();await dialog.getByRole('tab',{name:'文字处理'}).click();
 await modelAction(page,dialog,'编辑');await expect(dialog.getByLabel('API 密钥',{exact:true})).toHaveValue('');
 await manualModel(dialog);await dialog.getByLabel('模型',{exact:true}).fill('updated-model');
 await dialog.getByRole('button',{name:'关闭',exact:true}).click();
 const confirm=page.getByRole('alertdialog');await expect(confirm).toBeVisible();await hitVisible(confirm.getByRole('button',{name:'取消'}));
 await confirm.getByRole('button',{name:'取消'}).click();await expect(dialog.getByLabel('模型',{exact:true})).toHaveValue('updated-model');
 await page.keyboard.press('Escape');await confirm.getByRole('button',{name:'保存'}).click();await expect(dialog).toHaveCount(0);
 await expect(page.getByRole('button',{name:'配置模型',exact:true})).toBeFocused();
});
test('necessary fields and fixed actions remain clickable at both window sizes',async({page})=>{
 for(const size of [{width:1120,height:760},{width:960,height:640}]){
  await page.setViewportSize(size);const dialog=await open(page);
  await hitVisible(dialog.getByLabel('API 密钥'));await hitVisible(dialog.getByRole('button',{name:'保存',exact:true}));await hitVisible(dialog.getByRole('button',{name:'关闭',exact:true}));
  await expect(dialog.getByLabel('名称')).toHaveCount(0);await expect(dialog.getByLabel('服务地址（URL）')).toHaveCount(0);
  await page.screenshot({path:'tests/visual/settings-speech-'+size.width+'.png'});
  await dialog.getByRole('tab',{name:'文字处理'}).click();await hitVisible(dialog.getByLabel('API 密钥'));await hitVisible(dialog.getByLabel('模型',{exact:true}));await hitVisible(dialog.getByRole('button',{name:'保存',exact:true}));
  await page.screenshot({path:'tests/visual/settings-text-'+size.width+'.png'});
  await dialog.getByRole('button',{name:'更多设置'}).click();
  await dialog.getByTestId('model-scroll').evaluate(el=>{el.scrollTop=el.scrollHeight;});
  await hitVisible(dialog.getByRole('button',{name:'保存',exact:true}));await hitVisible(dialog.getByRole('button',{name:'关闭',exact:true}));
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  const green=await dialog.locator('*').evaluateAll(nodes=>nodes.filter(n=>Array.from(n.childNodes).some(c=>c.nodeType===3&&c.textContent?.trim())&&getComputedStyle(n).color==='rgb(117, 251, 145)').length);expect(green).toBe(0);
  await page.keyboard.press('Escape');await expect(dialog).toHaveCount(0);
 }
});
test('close confirms visible changes from the bottom and cancels category changes safely',async({page})=>{
 const dialog=await open(page);await dialog.getByRole('button',{name:'更多设置'}).click();await dialog.getByLabel('名称').fill('我的语音');
 await dialog.getByTestId('model-scroll').evaluate(el=>{el.scrollTop=el.scrollHeight;});
 await dialog.getByRole('tab',{name:'文字处理'}).click();const confirm=page.getByRole('alertdialog');await hitVisible(confirm.getByRole('button',{name:'取消'}));
 expect(await page.locator('[role="dialog"] button[aria-label="关闭"]').evaluate(el=>{const r=el.getBoundingClientRect();return !el.contains(document.elementFromPoint(r.left+r.width/2,r.top+r.height/2));})).toBe(true);
 await confirm.getByRole('button',{name:'取消'}).click();await expect(dialog.getByRole('tab',{name:'语音识别'})).toHaveAttribute('aria-selected','true');
 await page.keyboard.press('Escape');await confirm.getByRole('button',{name:'保存'}).click();await expect(dialog).toBeVisible();await expect(dialog.getByLabel('API 密钥')).toBeFocused();
 await dialog.getByLabel('API 密钥').fill('fixture-speech-key');await page.keyboard.press('Escape');await confirm.getByRole('button',{name:'保存'}).click();await expect(dialog).toHaveCount(0);
});
test('empty keys focus the error and pristine close works through button and backdrop',async({page})=>{
 let dialog=await open(page);await dialog.getByRole('button',{name:'关闭',exact:true}).click();await expect(dialog).toHaveCount(0);
 dialog=await open(page);await page.mouse.click(8,8);await expect(dialog).toHaveCount(0);
 dialog=await open(page);await dialog.getByRole('button',{name:'保存',exact:true}).click();await expect(dialog.getByLabel('API 密钥')).toBeFocused();await expect(dialog.getByLabel('API 密钥')).toHaveAttribute('aria-invalid','true');
 await hitVisible(dialog.getByRole('button',{name:'关闭',exact:true}));await dialog.getByRole('button',{name:'关闭',exact:true}).click();await expect(dialog).toHaveCount(0);
});
test('long model summaries, menus and delete confirmation stay within a small window',async({page})=>{
 await page.setViewportSize({width:960,height:640});const dialog=await open(page);
 await dialog.getByRole('combobox',{name:'模型',exact:true}).click();
 const option=page.getByRole('option',{name:'Qwen Audio 3.0 · 实时识别',exact:true});await hitVisible(option);await page.screenshot({path:'tests/visual/settings-model-menu-960.png'});await page.keyboard.press('Escape');
 await dialog.getByRole('tab',{name:'文字处理'}).click();
 await dialog.getByLabel('服务地址（URL）',{exact:true}).fill('https://example.invalid/v1');await dialog.getByLabel('API 密钥',{exact:true}).fill('fixture-key');
 await manualModel(dialog);await dialog.getByLabel('模型',{exact:true}).fill('model-'+ 'abcdefghij'.repeat(23));await dialog.getByRole('button',{name:'更多设置'}).click();
 const name='这是一份需要完整保留名称的个人模型'.repeat(3);await dialog.getByLabel('名称',{exact:true}).fill(name);
 await dialog.getByRole('button',{name:'保存',exact:true}).click();await expect(dialog.locator('[data-model-card][data-selected=true]')).toBeVisible();
 expect(await dialog.evaluate(el=>el.scrollWidth<=el.clientWidth)).toBe(true);await hitVisible(dialog.getByRole('button',{name:'关闭',exact:true}));
 await modelAction(page,dialog,'删除');const confirm=page.getByRole('alertdialog');await hitVisible(confirm.getByRole('button',{name:'确认删除'}));
 expect(await confirm.evaluate(el=>el.scrollWidth<=el.clientWidth)).toBe(true);await page.keyboard.press('Escape');await expect(confirm).toHaveCount(0);
});


test('model category tabs move a shared indicator with stable buttons and keyboard semantics',async({page})=>{
 const dialog=await open(page), speech=dialog.getByRole('tab',{name:'语音识别'}), text=dialog.getByRole('tab',{name:'文字处理'});
 const group=dialog.getByRole('tablist',{name:'模型类别'}),before=await group.boundingBox();
 const indicator=group.locator('[data-rubber-indicator]'),buttonBounds=await text.boundingBox();
 await speech.focus();await page.keyboard.press('ArrowRight');await expect(text).toBeFocused();await expect(speech).toHaveAttribute('aria-selected','true');
 const movement=indicator.evaluate(async el=>{const samples=[el.getBoundingClientRect().x];for(let i=0;i<40;i++){await new Promise<void>(resolve=>requestAnimationFrame(()=>resolve()));samples.push(el.getBoundingClientRect().x);}return samples;});
 await page.keyboard.press('Enter');await expect(text).toHaveAttribute('aria-selected','true');
 const samples=await movement;expect(Math.max(...samples)-Math.min(...samples)).toBeGreaterThan(8);
 expect(await group.boundingBox()).toEqual(before);
 expect(await text.boundingBox()).toEqual(buttonBounds);
 await expect.poll(async()=>{const a=await indicator.boundingBox(),b=await text.boundingBox();return !!a&&!!b&&Math.abs(a.x-b.x)<1&&Math.abs(a.y-b.y)<1&&Math.abs(a.width-b.width)<1&&Math.abs(a.height-b.height)<1;}).toBe(true);
 await expect(indicator).toHaveCSS('background-color','rgb(0, 0, 0)');await expect(indicator).toHaveCSS('opacity','1');
 await expect(text.locator('[data-choice-face]')).toHaveCSS('background-color','rgba(0, 0, 0, 0)');
 await expect(text).toHaveCSS('color','rgb(246, 246, 246)');
 expect(await text.evaluate(el=>getComputedStyle(el).backgroundColor)).toBe('rgba(0, 0, 0, 0)');
 await hitVisible(dialog.getByLabel('服务地址（URL）'));await expect(dialog.getByLabel('模型',{exact:true})).toHaveText('选择文字模型');
 await page.keyboard.press('Escape');await expect(dialog).toHaveCount(0);
});

test('compact actions, selected surface and reduced motion remain clear',async({page})=>{
 await page.setViewportSize({width:960,height:640});await page.emulateMedia({reducedMotion:'reduce'});
 const dialog=await open(page),tabs=dialog.getByRole('tablist',{name:'模型类别'});
 await expect(tabs.getByRole('tab').first()).toHaveCSS('transition-duration','0s');
 const indicator=tabs.locator('[data-rubber-indicator]');await expect(indicator).toHaveCSS('background-color','rgb(0, 0, 0)');await expect(indicator).toHaveCSS('opacity','1');
 await dialog.getByRole('tab',{name:'文字处理'}).click();
 const selected=dialog.getByRole('tab',{name:'文字处理'});expect(await selected.evaluate(el=>new DOMMatrix(getComputedStyle(el).transform).isIdentity)).toBe(true);
 await expect.poll(async()=>{const a=await indicator.boundingBox(),b=await selected.boundingBox();return !!a&&!!b&&Math.abs(a.x-b.x)<1&&Math.abs(a.width-b.width)<1;}).toBe(true);
 const more=(await dialog.getByRole('button',{name:'更多设置'}).boundingBox())!,save=(await dialog.getByRole('button',{name:'保存',exact:true}).boundingBox())!;
 expect(save.y-(more.y+more.height)).toBeLessThan(80);await hitVisible(dialog.getByLabel('模型',{exact:true}));
 await page.screenshot({path:'tests/visual/settings-compact-960.png'});
 await page.keyboard.press('Escape');await expect(dialog).toHaveCount(0);
});
test('feature settings save immediately through their own entries',async({page})=>{
 await page.goto('/#/preferences');const personal=page.getByRole('switch',{name:'个性化'});await expect(personal).toBeChecked();await personal.click();await expect(personal).not.toBeChecked();await expect(page.getByRole('alertdialog')).toHaveCount(0);
 await page.getByRole('link',{name:'词典',exact:true}).click();const automatic=page.getByRole('switch',{name:'自动入词'});await expect(automatic).toBeChecked();await automatic.click();
 await expect(page.getByRole('button',{name:'词典更多操作'})).toHaveCount(0);await expect(automatic).toBeVisible();await expect(automatic).not.toBeChecked();
 await page.getByRole('link',{name:'个性化',exact:true}).click();await expect(page.getByRole('switch',{name:'个性化'})).not.toBeChecked();await page.getByRole('button',{name:'设置',exact:true}).click();const reopened=page.getByRole('dialog',{name:'设置'});
 await reopened.getByRole('button',{name:'模型',exact:true}).click();await reopened.getByRole('tab',{name:'文字处理'}).click();
 await reopened.getByLabel('API 密钥').fill('fixture-key');await manualModel(reopened);await reopened.getByLabel('模型',{exact:true}).fill('fixture-text');await reopened.getByRole('button',{name:'保存',exact:true}).click();
 const polish=reopened.getByRole('switch',{name:'整理口述'});await expect(polish).toBeChecked();await expect(reopened.getByRole('switch')).toHaveCount(1);await polish.click();await reopened.getByRole('button',{name:'关闭',exact:true}).click();await expect(page.getByRole('alertdialog')).toHaveCount(0);
 await page.getByRole('link',{name:'个性化',exact:true}).click();await expect(page.getByRole('switch',{name:'个性化'})).not.toBeChecked();await page.getByRole('button',{name:'设置',exact:true}).click();const preferences=page.getByRole('dialog',{name:'设置'});await preferences.getByRole('button',{name:'模型',exact:true}).click();await preferences.getByRole('tab',{name:'文字处理'}).click();await expect(preferences.getByRole('switch',{name:'整理口述'})).not.toBeChecked();
});

test('synchronized text model picker stays visible, searches and restores focus',async({page})=>{
 for(const size of [{width:1120,height:760},{width:960,height:640}]){
  await page.setViewportSize(size);const settings=await open(page);
  await settings.getByRole('tab',{name:'文字处理'}).click();
  await settings.getByLabel('API 密钥').fill('fixture-key');
  const model=settings.getByLabel('模型',{exact:true});await model.click();
  const picker=page.getByRole('dialog',{name:'选择文字模型',exact:true});
  await expect(picker.getByRole('option',{name:'demo-text-model'})).toBeVisible();
  await hitVisible(picker.getByLabel('搜索模型'));await hitVisible(picker.getByRole('button',{name:'刷新'}));
  await hitVisible(picker.getByRole('button',{name:'关闭',exact:true}));
  await picker.getByLabel('搜索模型').fill('second');
  const option=picker.getByRole('option',{name:'demo-second-model'});await hitVisible(option);
  await option.focus();await page.keyboard.press('Enter');
  await expect(picker).toHaveCount(0);await expect(model).toBeFocused();await expect(model).toContainText('demo-second-model');
  await model.click();await page.keyboard.press('Escape');await expect(picker).toHaveCount(0);await expect(model).toBeFocused();
 }
});


for(const theme of ['light','graphite'])for(const width of [1120,960])test(`model cards keep geometry and independent floating menus ${theme} ${width}`,async({page})=>{
 await page.setViewportSize({width,height:width===960?640:760});
 await page.goto('/');await page.getByRole('button',{name:'设置',exact:true}).click();
 const dialog=page.getByRole('dialog',{name:'设置',exact:true});
 await dialog.getByRole('button',{name:'外观',exact:true}).click();await dialog.getByRole('button',{name:theme==='light'?'浅色':'石墨银灰',exact:true}).click();
 await expect(page.locator('html')).toHaveAttribute('data-theme',theme);
 await dialog.getByRole('button',{name:'模型',exact:true}).click();
 const names=['日常云端识别','备用云端识别 · 用于检查较长名称的显示与省略'];
 for(const [i,name] of names.entries()){
  if(i)await dialog.getByRole('button',{name:'添加模型',exact:true}).click();
  await dialog.getByLabel('API 密钥',{exact:true}).fill('fixture-key');await dialog.getByRole('button',{name:'更多设置',exact:true}).click();
  await dialog.getByLabel('名称',{exact:true}).fill(name);await dialog.getByRole('button',{name:'保存',exact:true}).click();
  await expect(dialog.getByRole('button',{name,exact:true})).toBeVisible();
 }
 const local=dialog.getByRole('region',{name:'本地语音模型'}),chooseLocal=local.getByRole('button',{name:'Nemotron 3.5 · 本地流式识别',exact:true});
 await local.getByRole('button',{name:'下载',exact:true}).click();await expect(chooseLocal).toBeEnabled();await expect(chooseLocal).toHaveAttribute('aria-pressed','false');
 const cards=dialog.locator('[data-model-card]');await expect(cards).toHaveCount(3);
 const modelGroup=dialog.getByRole('group',{name:'语音识别模型',exact:true}),modelPlate=modelGroup.locator(':scope > [data-rubber-indicator]');
 const plateAligned=()=>expect.poll(async()=>{const a=await modelPlate.boundingBox(),b=await chooseLocal.locator('..').boundingBox();return !!a&&!!b&&Math.abs(a.y-b.y)<.1&&Math.abs(a.height-b.height)<.1;}).toBe(true);
 const boxes=()=>cards.evaluateAll(elements=>elements.map(el=>{const r=el.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height};}));
 const before=await boxes();for(const box of before){expect(box.height).toBe(80);expect(box.width).toBe(before[0]!.width);}
 const homeButton=dialog.getByRole('button',{name:'去首页试说',exact:true}),footerBefore=await homeButton.boundingBox(),scrollBefore=await dialog.getByTestId('model-scroll').evaluate(el=>el.scrollTop);
 const first=dialog.getByRole('button',{name:names[0]!,exact:true}),second=dialog.getByRole('button',{name:names[1]!,exact:true});
 await expect(first).toHaveAttribute('aria-pressed','true');
 await cards.first().hover();const more=cards.first().getByRole('button',{name:/^更多操作 /u});await expect(more.locator('..')).toHaveCSS('opacity','1');
 await more.click();const menu=page.getByRole('menu');await expect(menu).toBeVisible();
 const edit=menu.getByRole('menuitem',{name:'编辑',exact:true}),remove=menu.getByRole('menuitem',{name:'删除',exact:true});
 await edit.hover();await highlightAt(menu,edit);await remove.hover();await highlightAt(menu,remove);
 await expect(menu.locator('[data-selection-highlight]')).toHaveCSS('transition-duration','0.22s');
 await edit.hover();await remove.hover();await edit.hover();await highlightAt(menu,edit);
 expect(await boxes()).toEqual(before);expect(await menu.evaluate(el=>!!el.closest('[data-model-card]'))).toBe(false);
 const box=(await menu.boundingBox())!;expect(box.x).toBeGreaterThanOrEqual(0);expect(box.y).toBeGreaterThanOrEqual(0);expect(box.x+box.width).toBeLessThanOrEqual(width);expect(box.y+box.height).toBeLessThanOrEqual(width===960?640:760);
 await page.screenshot({path:`.local/model-selection-motion/visuals/${theme}-${width}-menu.png`});
 await second.click();await expect(menu).toHaveCount(0);await expect(second).toHaveAttribute('aria-pressed','true');await expect(first).toHaveAttribute('aria-pressed','false');expect(await boxes()).toEqual(before);
 await local.hover();await local.getByRole('button',{name:/^更多操作 /u}).click();await expect(page.getByRole('menuitem',{name:'删除模型',exact:true})).toBeVisible();
 const localMenu=page.getByRole('menu'),bounds=(await dialog.boundingBox())!,floating=(await localMenu.boundingBox())!;
 expect(floating.x).toBeGreaterThanOrEqual(bounds.x);expect(floating.y).toBeGreaterThanOrEqual(bounds.y);
 expect(floating.x+floating.width).toBeLessThanOrEqual(bounds.x+bounds.width);expect(floating.y+floating.height).toBeLessThanOrEqual(bounds.y+bounds.height);
 await hitVisible(page.getByRole('menuitem',{name:'计算方式',exact:true}));await hitVisible(page.getByRole('menuitem',{name:'删除模型',exact:true}));expect(await boxes()).toEqual(before);
 await dialog.getByRole('heading',{name:'已保存模型',exact:true}).click();await expect(page.getByRole('menu')).toHaveCount(0);await expect(chooseLocal).toHaveAttribute('aria-pressed','false');
 const oldY=(await modelPlate.boundingBox())!.y,goalY=(await chooseLocal.locator('..').boundingBox())!.y;
 const trace=modelPlate.evaluate(async el=>{const positions:number[]=[];for(let i=0;i<45;i++){await new Promise(requestAnimationFrame);positions.push(el.getBoundingClientRect().y);}return positions;});
 await chooseLocal.click();await expect(chooseLocal).toHaveAttribute('aria-pressed','true');await expect(chooseLocal).toBeEnabled();await expect(first).toBeEnabled();expect(await boxes()).toEqual(before);
 const positions=await trace;expect(positions.some(y=>y>Math.min(oldY,goalY)+1&&y<Math.max(oldY,goalY)-1)).toBe(true);
 await plateAligned();await expect(modelPlate).toHaveCSS('background-color',theme==='light'?'rgb(0, 0, 0)':'rgb(222, 223, 226)');
 const inverseCopy=chooseLocal.locator('[aria-hidden="true"]').last();await expect(inverseCopy).toHaveCSS('color',theme==='light'?'rgb(246, 246, 246)':'rgb(25, 26, 28)');
 expect(await second.locator('..').evaluate(el=>getComputedStyle(el,'::before').backgroundColor)).toBe(theme==='light'?'rgb(246, 246, 246)':'rgb(25, 26, 28)');
 expect(await homeButton.boundingBox()).toEqual(footerBefore);expect(await dialog.getByTestId('model-scroll').evaluate(el=>el.scrollTop)).toBe(scrollBefore);
 await expect(dialog.getByText('本机识别；文字整理按所选文字模型处理。',{exact:true})).toHaveCount(0);
 await first.click();await expect(first).toHaveAttribute('aria-pressed','true');await expect(first).toBeEnabled();expect(await boxes()).toEqual(before);expect(await homeButton.boundingBox()).toEqual(footerBefore);
 await chooseLocal.click();await expect(chooseLocal).toHaveAttribute('aria-pressed','true');await expect(chooseLocal).toBeEnabled();await plateAligned();
 await page.screenshot({path:`.local/model-selection-motion/visuals/${theme}-${width}-selected.png`});
 await page.emulateMedia({reducedMotion:'reduce'});await expect(modelGroup).toHaveAttribute('data-motion','static');await expect(modelPlate).toHaveCSS('transition-property','none');await first.click();await expect(first).toHaveAttribute('aria-pressed','true');await expect(first).toBeEnabled();
 const reducedPlate=await modelPlate.boundingBox(),reducedCard=await first.locator('..').boundingBox();expect(reducedPlate?.y).toBe(reducedCard?.y);expect(reducedPlate?.height).toBe(80);expect(await boxes()).toEqual(before);expect(await homeButton.boundingBox()).toEqual(footerBefore);
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
});
