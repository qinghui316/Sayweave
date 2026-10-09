import {test,expect} from '@playwright/test';
test.use({deviceScaleFactor:1.5});
test('personalization and automatic terms are findable at both desktop sizes',async({page})=>{
 for(const [width,height] of [[1120,760],[960,640]] as const){
  await page.setViewportSize({width,height});await page.goto('/');await page.getByRole('button',{name:'设置',exact:true}).click();
  const dialog=page.getByRole('dialog',{name:'设置'});await dialog.getByRole('button',{name:'个性化',exact:true}).click();
  await expect(dialog.getByRole('switch',{name:'个性化'})).toBeEnabled();await expect(dialog.getByRole('heading',{name:'个性化',exact:true,level:2})).toBeVisible();
  expect(await dialog.evaluate(node=>{const box=node.getBoundingClientRect();return box.left>=0&&box.top>=0&&box.right<=innerWidth&&box.bottom<=innerHeight;})).toBe(true);
  await page.screenshot({path:'.local/settings-personalization-'+width+'.png'});await dialog.getByRole('button',{name:'关闭',exact:true}).click();
  await page.getByRole('link',{name:'词典',exact:true}).click();await expect(page.getByRole('switch',{name:'自动入词'})).toBeEnabled();
  await page.getByRole('tab',{name:'常见纠错'}).click();await expect(page.getByRole('switch',{name:'自动入词'})).toBeVisible();
  await expect(page.getByRole('tab',{name:'常见纠错'})).toHaveCSS('color','rgb(246, 246, 246)');
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);await page.screenshot({path:'.local/settings-dictionary-'+width+'.png'});
 }
});
test('the preferences route opens the same settings entry and closes directly',async({page})=>{
 await page.goto('/#/preferences');const dialog=page.getByRole('dialog',{name:'设置'});await expect(dialog.getByRole('switch',{name:'个性化'})).toBeEnabled();
 await dialog.getByRole('switch',{name:'个性化'}).click();await dialog.getByRole('button',{name:'关闭',exact:true}).click();await expect(dialog).toHaveCount(0);await expect(page.getByRole('alertdialog')).toHaveCount(0);await expect(page.getByRole('heading',{name:'开始口述'})).toBeVisible();
});
