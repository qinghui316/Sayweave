import {test,expect} from '@playwright/test';
test('models configure inline, distinguish simulation and persist within renderer lifetime',async({page})=>{
 await page.goto('/');await page.getByRole('button',{name:'配置模型',exact:true}).click();
 const dialog=page.getByRole('dialog');await expect(dialog.getByText(/浏览器模拟/)).toBeVisible();
 await dialog.getByLabel('配置名称',{exact:true}).fill('个人配置');
 await dialog.getByLabel('服务基址',{exact:true}).fill('https://example.invalid/v1');
 await dialog.getByLabel('API Key',{exact:true}).fill('fixture-key');
 await dialog.getByRole('button',{name:'获取模型列表'}).click();await expect(dialog.getByText('列表已获取；选择或手动填写模型 ID。')).toBeVisible();
 await dialog.getByLabel('模型 ID',{exact:true}).fill('demo-text-model');
 await dialog.getByRole('button',{name:'测试文字模型'}).click();await expect(dialog.getByText(/模拟测试完成/)).toBeVisible();
 await dialog.getByRole('button',{name:'保存配置'}).click();await expect(dialog.getByText('配置已保存。')).toBeVisible();
 await expect(dialog.getByLabel('文字处理默认配置',{exact:true})).not.toHaveValue('');
 await page.keyboard.press('Escape');await expect(dialog).toHaveCount(0);
 await page.getByRole('button',{name:'配置模型',exact:true}).click();await expect(dialog.getByRole('button',{name:/个人配置.*demo-text-model/})).toBeVisible();
 await dialog.getByRole('button',{name:/个人配置.*demo-text-model/}).click();await expect(dialog.getByLabel('API Key',{exact:true})).toHaveValue('');
 await dialog.getByLabel('配置名称',{exact:true}).fill('有修改');
 await page.keyboard.press('Escape');await expect(dialog.getByRole('alertdialog')).toContainText('有未保存的修改');
 await dialog.getByRole('button',{name:'继续编辑'}).click();await expect(dialog.getByLabel('配置名称',{exact:true})).toHaveValue('有修改');
 await page.keyboard.press('Escape');await dialog.getByRole('button',{name:'放弃修改'}).click();await expect(dialog).toHaveCount(0);
 await expect(page.getByRole('button',{name:'配置模型',exact:true})).toBeFocused();
});
test('model panel scrolls at compact sizes with neutral text',async({page})=>{
 for(const size of [{width:1120,height:760},{width:960,height:640}]){
  await page.setViewportSize(size);await page.goto('/');await page.getByRole('button',{name:'配置模型',exact:true}).click();
  const dialog=page.getByRole('dialog');await expect(dialog.getByLabel('配置名称',{exact:true})).toBeVisible();
  await dialog.getByRole('button',{name:'保存配置'}).scrollIntoViewIfNeeded();await expect(dialog.getByRole('button',{name:'保存配置'})).toBeVisible();
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  const green=await dialog.locator('*').evaluateAll(nodes=>nodes.filter(n=>Array.from(n.childNodes).some(c=>c.nodeType===3&&c.textContent?.trim())&&getComputedStyle(n).color==='rgb(117, 251, 145)').length);expect(green).toBe(0);
  await page.screenshot({path:'tests/visual/models-'+size.width+'.png'});
 }
});
