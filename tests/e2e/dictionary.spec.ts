import {test,expect,type Page} from '@playwright/test';
async function menu(page:Page,word:string,action:string){await page.getByRole('button',{name:'更多操作 '+word,exact:true}).click();await page.getByRole('menuitem',{name:action,exact:true}).click();}
test('simple words and advanced corrections preserve CRUD and keyboard focus',async({page})=>{
 await page.goto('/#/dictionary');await expect(page.getByText('还没有词条')).toBeVisible();await expect(page.getByRole('table')).toHaveCount(0);await expect(page.getByRole('tablist')).toHaveCount(0);
 await page.getByRole('button',{name:'添加词语'}).click();const dialog=page.getByRole('dialog');await expect(dialog.getByRole('textbox')).toHaveCount(1);await dialog.getByLabel('词语',{exact:true}).fill('Figma');await dialog.getByRole('button',{name:'保存',exact:true}).click();await expect(page.getByText('Figma',{exact:true})).toBeVisible();
 await menu(page,'Figma','编辑');await dialog.getByRole('button',{name:'补充设置'}).click();await dialog.getByLabel('分类（可选）').fill('设计工具');await dialog.getByRole('button',{name:'保存',exact:true}).click();
 await page.getByRole('button',{name:'添加词语'}).click();await dialog.getByLabel('词语',{exact:true}).fill('figma');await dialog.getByRole('button',{name:'保存',exact:true}).click();await expect(dialog.getByRole('alert')).toBeVisible();await dialog.getByRole('button',{name:'查看已有词条'}).click();await expect(dialog.getByLabel('词语',{exact:true})).toHaveValue('Figma');await dialog.getByLabel('词语',{exact:true}).fill('Figma Design');await dialog.getByRole('button',{name:'保存',exact:true}).click();
 await page.getByRole('button',{name:'搜索词语'}).click();await page.getByRole('textbox',{name:'搜索词条'}).fill('设计');await expect(page.getByText('Figma Design',{exact:true})).toBeVisible();await page.getByRole('textbox',{name:'搜索词条'}).fill('不存在');await expect(page.getByText('没有匹配的词条')).toBeVisible();await page.getByRole('textbox',{name:'搜索词条'}).fill('');
 await menu(page,'Figma Design','编辑');await dialog.getByRole('button',{name:'取消',exact:true}).click();await expect(page.getByRole('button',{name:'更多操作 Figma Design'})).toBeFocused();
 await page.getByRole('button',{name:'词典更多操作'}).click();await page.getByRole('menuitem',{name:'纠错映射'}).click();await page.getByRole('button',{name:'添加纠错'}).click();await dialog.getByLabel('听成').fill('飞格玛');await dialog.getByLabel('改为').fill('Figma');await dialog.getByRole('button',{name:'保存',exact:true}).click();await expect(page.getByRole('button',{name:'编辑词语 飞格玛',exact:true})).toBeVisible();
 await menu(page,'飞格玛','删除');await dialog.getByRole('button',{name:'取消',exact:true}).click();await expect(page.getByRole('button',{name:'编辑词语 飞格玛',exact:true})).toBeVisible();await menu(page,'飞格玛','删除');await dialog.getByRole('button',{name:'删除',exact:true}).click();await expect(page.getByText('还没有词条')).toBeVisible();
 await page.getByRole('button',{name:'词典更多操作'}).click();await page.getByRole('menuitem',{name:'返回词典'}).click();await expect(page.getByText('Figma Design',{exact:true})).toBeVisible();await page.reload();await expect(page.getByText('还没有词条')).toBeVisible();
});
test('compact dictionary and real-zero dashboard fit desktop sizes',async({page})=>{
 for(const [width,height] of [[1120,760],[960,640]] as const){
  await page.setViewportSize({width,height});await page.goto('/#/dictionary');await expect(page.getByRole('switch',{name:'自动入词'})).toBeEnabled();
  const bounds=await page.getByRole('switch',{name:'自动入词'}).boundingBox();expect(bounds!.y).toBeLessThan(130);await page.screenshot({path:'.local/dictionary-simple-'+width+'.png'});
  await page.getByRole('link',{name:'个性化',exact:true}).click();await expect(page.getByText('学习记录',{exact:true})).toBeVisible();await expect(page.getByText('使用一段时间后，这里会显示学到的表达习惯。')).toBeVisible();
  await page.screenshot({path:'.local/personalization-dashboard-'+width+'.png'});expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
 }
});
test('search expands in place and animated filters are not clipped',async({page})=>{
 await page.setViewportSize({width:960,height:640});await page.goto('/#/dictionary');
 await expect(page.getByRole('textbox',{name:'搜索词条'})).toHaveCount(0);const search=page.getByRole('button',{name:'搜索词语'});await search.click();const field=page.getByRole('textbox',{name:'搜索词条'});await expect(field).toBeFocused();
 const box=(await field.boundingBox())!;expect(box.width).toBeLessThan(220);expect(box.x).toBeGreaterThan(500);
 await field.fill('不存在');await page.getByRole('heading',{name:'词典',exact:true}).click();await expect(field).toBeVisible();await field.focus();await page.keyboard.press('Escape');await expect(field).toHaveCount(0);await expect(search).toBeFocused();
 for(const name of ['自动添加','手动添加','所有']){
  await page.getByRole('radio',{name,exact:true}).click();
  for(const delay of [40,100,180]){await page.waitForTimeout(delay);const clear=await page.getByRole('radiogroup',{name:'词条来源'}).evaluate(group=>Array.from(group.querySelectorAll('[data-choice-face]')).every(face=>{const r=face.getBoundingClientRect();for(let parent=face.parentElement;parent;parent=parent.parentElement){const css=getComputedStyle(parent);if(['hidden','auto','scroll','clip'].includes(css.overflowX)||['hidden','auto','scroll','clip'].includes(css.overflowY)){const bound=parent.getBoundingClientRect();if(r.left<bound.left-.5||r.right>bound.right+.5||r.top<bound.top-.5||r.bottom>bound.bottom+.5)return false;}}return r.left>=0&&r.right<=innerWidth;}));expect(clear).toBe(true);}
 }
 await page.screenshot({path:'.local/dictionary-search-filters-960.png'});
});
