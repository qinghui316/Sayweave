import {test,expect,type Locator} from '@playwright/test';
import type {HistoryRecord} from '../../src/contracts/history';

async function aligned(menu:Locator,item:Locator){
 const layer=menu.locator('[data-selection-highlight]');await expect(layer).toHaveCSS('opacity','1');
 await expect.poll(async()=>{const a=await layer.boundingBox(),b=await item.boundingBox();return !!a&&!!b&&Math.abs(a.x-b.x)<1&&Math.abs(a.y-b.y)<1&&Math.abs(a.height-b.height)<1;}).toBe(true);
}

test('dictionary menu hover and keyboard highlight preserve item positions and outside dismissal',async({page})=>{
 await page.goto('/#/dictionary');await page.getByRole('button',{name:'添加词语',exact:true}).click();
 const dialog=page.getByRole('dialog');await dialog.getByLabel('词语',{exact:true}).fill('交互核对');await dialog.getByRole('button',{name:'保存',exact:true}).click();
 const word=page.getByText('交互核对',{exact:true}),before=await word.boundingBox();await page.getByRole('button',{name:'更多操作 交互核对',exact:true}).click();
 const menu=page.getByRole('menu'),edit=menu.getByRole('menuitem',{name:'编辑',exact:true}),remove=menu.getByRole('menuitem',{name:'删除',exact:true});
 await edit.hover();await aligned(menu,edit);const bounds=await edit.boundingBox();await remove.hover();await aligned(menu,remove);
 await expect(menu.locator('[data-selection-highlight]')).toHaveCSS('transition-duration','0.22s');expect(await edit.boundingBox()).toEqual(bounds);
 await page.keyboard.press('ArrowUp');await expect(edit).toBeFocused();await aligned(menu,edit);expect(await word.boundingBox()).toEqual(before);
 await page.evaluate(()=>{const state=window as typeof window&{menuExits?:number[]};state.menuExits=[];document.addEventListener('animationend',event=>{if(event.target instanceof HTMLElement&&event.target.getAttribute('role')==='menu'&&event.animationName.includes('select-exit'))state.menuExits!.push(event.elapsedTime);});});
 await page.getByRole('heading',{name:'词典',exact:true}).click();await expect(menu).toHaveCount(0);await expect(word).toBeVisible();
 await expect.poll(()=>page.evaluate(()=>(window as typeof window&{menuExits?:number[]}).menuExits)).toEqual([.12]);
 const trigger=page.getByRole('button',{name:'更多操作 交互核对',exact:true});await trigger.click();await page.getByRole('heading',{name:'词典',exact:true}).click();await trigger.click();
 await edit.hover();await aligned(menu,edit);await edit.click();
 await expect(trigger).toBeDisabled();const target=(await trigger.boundingBox())!;await page.mouse.click(target.x+target.width/2,target.y+target.height/2);
 const editor=page.getByRole('dialog');await expect(editor.getByLabel('词语',{exact:true})).toHaveValue('交互核对');await expect(page.getByRole('menu')).toHaveCount(0);
 await editor.getByRole('button',{name:'取消',exact:true}).click();await expect(trigger).toBeEnabled();
});

test('history menu excludes unavailable actions and reduced motion keeps an immediate highlight',async({page})=>{
 const at=Date.now(),record:HistoryRecord={id:'menu-hover-record',createdAt:at,endedAt:at+1000,durationMs:1000,status:'success',currentAttempt:'attempt-hover',revision:1,audio:null,attempts:[{id:'attempt-hover',recordId:'menu-hover-record',kind:'live',configId:'fixture',configName:'测试模型',configRevision:1,modelId:'fixture',serviceHost:'example.invalid',createdAt:at,endedAt:at+1000,status:'success',text:'只用于菜单交互的历史正文。',errorCode:null,elapsedMs:1}]};
 await page.emulateMedia({reducedMotion:'reduce'});
 await page.route('**/src/adapters/speech-browser.ts',async route=>{const response=await route.fetch(),source=await response.text();expect(source).toContain('records = [];');await route.fulfill({response,body:source.replace('records = [];',`records = ${JSON.stringify([record])};`)});});
 await page.goto('/#/history');const row=page.getByRole('listitem').first();await row.hover();await row.getByRole('button',{name:'更多操作',exact:true}).click();
 const menu=page.getByRole('menu'),disabled=menu.getByRole('menuitem',{name:'重新识别',exact:true}),versions=menu.getByRole('menuitem',{name:'识别版本',exact:true});
 await expect(disabled).toHaveAttribute('aria-disabled','true');await versions.hover();await aligned(menu,versions);await expect(menu).toHaveCSS('animation-name','none');
 await expect(menu.locator('[data-selection-highlight]')).toHaveCSS('transition-duration','0s');await disabled.hover();await expect(disabled).not.toHaveAttribute('data-highlighted');await aligned(menu,versions);
 await page.getByRole('heading',{name:'历史记录',exact:true}).click();await expect(menu).toHaveCount(0);await expect(row).toContainText(record.attempts[0]!.text);
});
