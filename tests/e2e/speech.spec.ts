import {test,expect} from '@playwright/test';
test('simulated speech saves only completed sessions and real UI result is read-only',async({page})=>{
 await page.goto('/');await page.getByRole('button',{name:'开始录音',exact:true}).click();await expect(page.locator('[data-phase="recording"]')).toBeVisible();
 await page.getByRole('button',{name:'结束录音',exact:true}).click();await expect(page.locator('[data-phase="completed"]')).toBeVisible();
 await expect(page.getByRole('region',{name:'识别文字',exact:true})).toContainText('未访问麦克风或语音服务');
 await expect(page.getByRole('region',{name:'识别文字',exact:true})).not.toHaveAttribute('contenteditable');
 await page.getByRole('link',{name:'历史记录',exact:true}).click();await expect(page.getByText('1 条',{exact:true})).toBeVisible();
 await page.getByRole('button',{name:/这是浏览器模拟文字/}).click();const dialog=page.getByRole('dialog');await expect(dialog.getByRole('region',{name:'历史识别文字'})).toContainText('浏览器模拟');
 await expect(dialog.getByRole('button',{name:'重新识别',exact:true})).toBeDisabled();
 await dialog.getByRole('button',{name:'删除记录',exact:true}).click();await dialog.getByRole('button',{name:'取消',exact:true}).click();await expect(dialog).toBeVisible();
 await dialog.getByRole('button',{name:'删除记录',exact:true}).click();await dialog.getByRole('button',{name:'确认删除'}).click();await expect(page.getByText('还没有识别记录')).toBeVisible();
 await page.getByRole('link',{name:'返回首页'}).click();await page.getByRole('button',{name:'开始录音',exact:true}).click();await expect(page.locator('[data-phase="recording"]')).toBeVisible();await page.locator('[data-phase]').getByRole('button',{name:'取消并丢弃',exact:true}).click();await page.getByRole('link',{name:'历史记录',exact:true}).click();await expect(page.getByText('0 条',{exact:true})).toBeVisible();
});
test('Qwen preset remains separate from fetched text models and cannot select text use',async({page})=>{
 await page.goto('/');await page.getByRole('button',{name:'配置模型',exact:true}).click();const dialog=page.getByRole('dialog');
 await dialog.getByLabel('接口类型').click();await page.getByRole('option',{name:'通义语音 · Qwen Audio Streaming'}).click();await expect(dialog.getByLabel('模型 ID',{exact:true})).toHaveValue('qwen-audio-3.1-asr-flash-streaming');
 await expect(dialog.getByRole('button',{name:'测试文字模型'})).toBeDisabled();await expect(dialog.getByRole('checkbox',{name:'文字处理',exact:true})).toHaveCount(0);
 await dialog.getByLabel('配置名称',{exact:true}).fill('通义语音');await dialog.getByLabel('服务基址',{exact:true}).fill('https://workspace.cn-beijing.maas.aliyuncs.com');await dialog.getByLabel('API Key',{exact:true}).fill('fixture-key');await dialog.getByRole('button',{name:'保存配置'}).click();
 await expect(dialog.getByLabel('语音识别默认配置',{exact:true})).not.toHaveValue('');await expect(dialog.getByLabel('文字处理默认配置',{exact:true})).toHaveValue('');
});
test('default development overlay follows speech state and speech actions together',async({page})=>{
 await page.goto('/?surface=overlay');await page.getByRole('button',{name:'开始录音',exact:true}).click();await expect(page.locator('[data-phase="recording"]')).toBeVisible();await page.getByRole('button',{name:'结束录音',exact:true}).click();await expect(page.locator('[data-phase="completed"]')).toBeVisible();
});
