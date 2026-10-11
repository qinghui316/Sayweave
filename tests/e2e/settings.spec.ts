import {test,expect} from '@playwright/test';
test.use({deviceScaleFactor:1.5});
test('personalization and automatic terms are findable at both desktop sizes',async({page})=>{
 for(const [width,height] of [[1120,760],[960,640]] as const){
  await page.setViewportSize({width,height});await page.goto('/#/preferences');
  await expect(page.getByRole('switch',{name:'个性化'})).toBeEnabled();await expect(page.getByRole('heading',{name:'个性化',exact:true,level:1})).toBeVisible();
  await expect(page.getByRole('heading',{name:'个性化',exact:true})).toHaveCount(1);
  expect(await page.getByRole('switch',{name:'个性化'}).evaluate(node=>innerWidth-node.getBoundingClientRect().right)).toBeLessThanOrEqual(40);
  await expect(page.getByRole('dialog')).toHaveCount(0);await expect(page.getByRole('link',{name:'个性化'})).toHaveAttribute('aria-current','page');
  const footer=page.locator('footer');await expect(footer.getByRole('button')).toHaveCount(2);await expect(footer.getByRole('button',{name:'关于'})).toBeVisible();
  expect(await footer.getByRole('button',{name:'设置',exact:true}).evaluate(node=>{const r=node.getBoundingClientRect();return r.height<=40&&node.scrollWidth<=node.clientWidth;})).toBe(true);
  await page.screenshot({path:'.local/settings-personalization-'+width+'.png'});
  await page.getByRole('link',{name:'词典',exact:true}).click();await expect(page.getByRole('switch',{name:'自动入词'})).toBeEnabled();
  await page.getByRole('button',{name:'词典更多操作'}).click();await page.getByRole('menuitem',{name:'纠错映射'}).click();await expect(page.getByRole('switch',{name:'自动入词'})).toBeVisible();
  await expect(page.getByRole('heading',{name:'纠错映射'})).toBeVisible();
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);await page.screenshot({path:'.local/settings-dictionary-'+width+'.png'});
 }
});
test('the standalone preferences route keeps saves across navigation and is absent in settings',async({page})=>{
 await page.goto('/#/preferences');const toggle=page.getByRole('switch',{name:'个性化'});await expect(toggle).toBeEnabled();await toggle.click();
 await page.getByRole('link',{name:'首页',exact:true}).click();await page.getByRole('link',{name:'个性化',exact:true}).click();await expect(toggle).not.toBeChecked();
 await page.getByRole('button',{name:'设置',exact:true}).click();const dialog=page.getByRole('dialog',{name:'设置'});await expect(dialog.getByRole('button',{name:'个性化',exact:true})).toHaveCount(0);
 await dialog.getByRole('button',{name:'关闭',exact:true}).click();await expect(page.getByRole('heading',{name:'个性化',level:1})).toBeVisible();await expect(page.getByRole('alertdialog')).toHaveCount(0);
});
