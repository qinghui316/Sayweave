import {test,expect} from '@playwright/test';
test('simulated speech saves only completed sessions and real UI result is read-only',async({page})=>{
 await page.goto('/');await page.getByRole('button',{name:'开始录音',exact:true}).click();await expect(page.locator('[data-phase="recording"]')).toBeVisible();
 await page.getByRole('button',{name:'结束录音',exact:true}).click();await expect(page.locator('[data-phase="completed"]')).toBeVisible();
 await expect(page.getByRole('region',{name:'识别文字',exact:true})).toContainText('未访问麦克风或语音服务');
 await expect(page.getByRole('region',{name:'识别文字',exact:true})).not.toHaveAttribute('contenteditable');
 await page.getByRole('link',{name:'历史记录',exact:true}).click();await expect(page.getByText('1 条',{exact:true})).toBeVisible();
 const row=page.getByRole('listitem').first();await row.hover();await row.getByRole('button',{name:'更多操作'}).click();await page.getByRole('menuitem',{name:'识别版本'}).click();
 await expect(row.getByRole('region',{name:'历史识别文字'})).toContainText('浏览器模拟');await expect(page.getByRole('dialog')).toHaveCount(0);
 await row.getByRole('button',{name:'更多操作'}).click();await expect(page.getByRole('menuitem',{name:'重新识别'})).toHaveAttribute('aria-disabled','true');await page.keyboard.press('Escape');
 await row.getByRole('button',{name:'更多操作'}).click();await page.getByRole('menuitem',{name:'删除记录'}).click();await row.getByRole('button',{name:'取消',exact:true}).click();await expect(row).toBeVisible();
 await row.getByRole('button',{name:'更多操作'}).click();await page.getByRole('menuitem',{name:'删除记录'}).click();await row.getByRole('button',{name:'确认删除'}).click();await expect(page.getByText('还没有历史记录')).toBeVisible();
 await page.getByRole('link',{name:'首页',exact:true}).click();await page.getByRole('button',{name:'开始录音',exact:true}).click();await expect(page.locator('[data-phase="recording"]')).toBeVisible();await page.locator('[data-phase]').getByRole('button',{name:'取消并丢弃',exact:true}).click();await page.getByRole('link',{name:'历史记录',exact:true}).click();await expect(page.getByText('0 条',{exact:true})).toBeVisible();
});
test('speech presets select concrete models and keep text fields independent',async({page})=>{
 await page.goto('/');await page.getByRole('button',{name:'配置模型',exact:true}).click();const dialog=page.getByRole('dialog');
 await expect(dialog.getByRole('tab',{name:'语音识别'})).toHaveAttribute('aria-selected','true');
 await expect(dialog.getByRole('combobox',{name:'模型'})).toContainText('Qwen Audio 3.1 · 实时识别');await dialog.getByRole('combobox',{name:'模型'}).click();await page.getByRole('option',{name:'Qwen Audio 3.0 · 实时识别'}).click();await expect(dialog.getByRole('button',{name:'测试连接'})).toHaveCount(0);
 await dialog.getByRole('button',{name:'更多设置'}).click();await expect(dialog.getByRole('combobox',{name:'模型'})).toContainText('Qwen Audio 3.0 · 实时识别');
 await dialog.getByLabel('服务地址（URL）',{exact:true}).fill('https://workspace.cn-beijing.maas.aliyuncs.com');await dialog.getByLabel('API 密钥',{exact:true}).fill('fixture-key');await dialog.getByRole('button',{name:'保存',exact:true}).click();
 await expect(dialog.getByText('当前使用',{exact:true})).toBeVisible();await dialog.getByRole('tab',{name:'文字处理'}).click();await expect(dialog.getByLabel('API 密钥',{exact:true})).toHaveValue('');
});
test('default development overlay follows speech state and speech actions together',async({page})=>{
 await page.goto('/?surface=overlay');await page.getByRole('button',{name:'开始录音',exact:true}).click();await expect(page.locator('[data-phase="recording"]')).toBeVisible();await page.getByRole('button',{name:'结束录音',exact:true}).click();await expect(page.locator('[data-phase="completed"]')).toBeVisible();
});
