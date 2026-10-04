import { test, expect } from '@playwright/test';
test('settings stays in one modal and restores focus',async({page})=>{
  await page.goto('/');await page.getByRole('button',{name:'配置模型',exact:true}).click();
  const dialog=page.getByRole('dialog');await expect(dialog).toBeVisible();await expect(dialog.getByRole('heading',{name:'模型',exact:true})).toBeVisible();
  await dialog.getByRole('button',{name:'关于',exact:true}).click();await expect(dialog.getByRole('heading',{name:'关于',exact:true})).toBeVisible();
  await page.screenshot({path:'tests/visual/settings-about.png'});
  await page.keyboard.press('Escape');await expect(dialog).not.toBeVisible();await expect(page.getByRole('button',{name:'配置模型',exact:true})).toBeFocused();
});
test('navigation geometry is stable and feature routes are explicit',async({page})=>{
  await page.goto('/');const link=page.getByRole('link',{name:'历史记录',exact:true});const before=await link.boundingBox();
  await link.click();expect(await link.boundingBox()).toEqual(before);await expect(page.getByText('还没有历史记录')).toBeVisible();
  await page.getByRole('link',{name:'首页',exact:true}).click();await expect(page.getByRole('heading',{name:'随言 — 随口表达，文字随言而来'})).toBeVisible();
});
test('developer demo remains isolated from production speech entry',async({page})=>{
  await page.goto('/');await page.getByRole('link',{name:'组件展示'}).click();await page.getByRole('button',{name:'开始预设演示'}).click();await expect(page.locator('[data-phase="recording"]')).toBeVisible();await page.getByRole('button',{name:'触发演示失败'}).click();await expect(page.locator('[data-phase="failed"]')).toBeVisible();await page.getByRole('button',{name:'重试演示'}).click();await expect(page.locator('[data-phase="recording"]')).toBeVisible();
});
test('home fits baseline/smaller layout; neutral text and local assets',async({page})=>{
  await page.goto('/');await page.evaluate(()=>document.fonts.ready);await page.screenshot({path:'tests/visual/home-1440.png'});
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  const green=await page.locator('body *').evaluateAll(nodes=>nodes.filter(n=>Array.from(n.childNodes).some(c=>c.nodeType===3&&c.textContent?.trim())&&getComputedStyle(n).color==='rgb(117, 251, 145)').map(n=>n.tagName));expect(green).toEqual([]);
  const rows=page.getByRole('heading',{level:3});const styles=await rows.evaluateAll(ns=>ns.map(n=>({size:getComputedStyle(n).fontSize,weight:getComputedStyle(n).fontWeight})));expect(new Set(styles.map(x=>JSON.stringify(x))).size).toBe(1);
  await page.setViewportSize({width:1280,height:800});await page.screenshot({path:'tests/visual/home-1280.png'});expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  await page.setViewportSize({width:1120,height:760});await page.screenshot({path:'tests/visual/home-1120.png'});expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  const trialButton=await page.getByRole('button',{name:'开始录音'}).boundingBox();expect(trialButton).not.toBeNull();expect(trialButton!.y+trialButton!.height).toBeLessThan(760);
  expect(await page.getByRole('heading',{level:1}).evaluate(n=>getComputedStyle(n).fontSize)).toBe('32px');
  await page.getByRole('button',{name:'配置模型',exact:true}).click();await page.screenshot({path:'tests/visual/settings-models-1120.png'});await page.keyboard.press('Escape');
  await page.setViewportSize({width:960,height:640});expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
});
test('search fields fit text and retain their focus ring at the minimum window',async({page})=>{
  for(const size of [{width:1120,height:760},{width:960,height:640}]){
    await page.setViewportSize(size);
    for(const [route,label] of [['/#/history','搜索识别文字'],['/#/dictionary','搜索词条']]){
      await page.goto(route!);const input=page.getByRole('textbox',{name:label!});await input.focus();
      const geometry=await input.evaluate(node=>{const css=getComputedStyle(node),box=node.getBoundingClientRect();return {textHeight:node.clientHeight-parseFloat(css.paddingTop)-parseFloat(css.paddingBottom),lineHeight:parseFloat(css.lineHeight),left:box.left,right:box.right,outlineExtent:parseFloat(css.outlineWidth)+parseFloat(css.outlineOffset),containerRight:node.parentElement!.getBoundingClientRect().right};});
      expect(geometry.textHeight).toBeGreaterThanOrEqual(geometry.lineHeight);
      expect(geometry.left).toBeGreaterThanOrEqual(0);expect(geometry.right).toBeLessThanOrEqual(geometry.containerRight);
      expect(geometry.outlineExtent).toBeLessThanOrEqual(0);await input.fill('较长的中文搜索内容，用于检查文字与按钮不会互相遮挡');await expect(input).toBeFocused();
      expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    }
  }
});
