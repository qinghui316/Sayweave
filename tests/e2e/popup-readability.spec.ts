import {test,expect,type Locator} from '@playwright/test';
async function readable(surface:Locator){
 await expect(surface).toBeVisible();
 await expect.poll(()=>surface.evaluate(root=>{
  type Color=[number,number,number,number];
  const rgba=(s:string):Color=>{const n=s.match(/[\d.]+/g)?.map(Number)??[0,0,0,0];return [n[0]??0,n[1]??0,n[2]??0,n[3]??1];};
  const blend=(a:Color,b:Color):Color=>[a[0]*a[3]+b[0]*(1-a[3]),a[1]*a[3]+b[1]*(1-a[3]),a[2]*a[3]+b[2]*(1-a[3]),1];
  const lum=(c:number[])=>c.slice(0,3).map(v=>{v/=255;return v<=.04045?v/12.92:((v+.055)/1.055)**2.4;}).reduce((s,v,i)=>s+v*([.2126,.7152,.0722][i]??0),0);
  return Array.from(root.querySelectorAll('*')).filter(e=>Array.from(e.childNodes).some(n=>n.nodeType===3&&n.textContent?.trim())).flatMap(e=>{
   const s=getComputedStyle(e),r=e.getBoundingClientRect();if(r.width<2||r.height<2||s.visibility==='hidden'||s.display==='none')return [];
   const ancestors:Element[]=[];for(let n:Element|null=e;n;n=n.parentElement)ancestors.unshift(n);
   let bg:Color=[255,255,255,1];for(const n of ancestors)bg=blend(rgba(getComputedStyle(n).backgroundColor),bg);
   // Selection backgrounds are sibling layers, not DOM ancestors of their labels.
   for(const layer of root.querySelectorAll('[data-rubber-indicator],[data-choice-face]')){
    const b=layer.getBoundingClientRect(),style=getComputedStyle(layer);
    if(Number(style.opacity)>0&&b.left<=r.left+r.width/2&&b.right>=r.left+r.width/2&&b.top<=r.top+r.height/2&&b.bottom>=r.top+r.height/2)bg=blend(rgba(style.backgroundColor),bg);
   }
   const color=rgba(s.color);color[3]*=Number(s.opacity);const fg=blend(color,bg),a=lum(fg),b=lum(bg),ratio=(Math.max(a,b)+.05)/(Math.min(a,b)+.05);
   return ratio<4.5?[`${e.textContent?.trim()}: ${ratio.toFixed(2)}`]:[];
  });
 })).toEqual([]);
}
for(const size of [{width:1120,height:760,scale:1},{width:960,height:640,scale:1.5}])test(`popup text and actions ${size.width}`,async({browser})=>{
 const context=await browser.newContext({viewport:size,deviceScaleFactor:size.scale,reducedMotion:'reduce'}),page=await context.newPage();
 await page.goto('/tests/fixtures/voice-recovery.html?surface=popups');
 await page.evaluate(()=>{document.body.style.color='white';});
 await page.getByRole('button',{name:'打开确认框'}).click();const confirm=page.getByRole('alertdialog');await readable(confirm);
 await expect(confirm.getByRole('button',{name:'处理中'})).toBeDisabled();
 await expect(confirm.getByRole('button',{name:'保存'}).locator('[aria-hidden]')).toHaveCSS('background-color','rgb(246, 246, 246)');
 await page.screenshot({path:`tests/visual/popup-confirm-${size.width}.png`});await confirm.getByRole('button',{name:'取消'}).click();
 await page.getByRole('combobox',{name:'测试选项'}).click();await readable(page.getByRole('listbox'));await page.keyboard.press('Escape');
 await page.getByRole('button',{name:'打开操作菜单'}).click();await readable(page.getByRole('menu'));await page.keyboard.press('Escape');
 await page.getByRole('button',{name:'model-a',exact:true}).click();const picker=page.getByRole('dialog',{name:'选择文字模型'});await readable(picker);await expect(picker.getByRole('heading',{name:'选择文字模型'})).toBeVisible();
 await page.screenshot({path:`tests/visual/popup-model-${size.width}.png`});await picker.getByRole('button',{name:'关闭',exact:true}).click();
 await page.goto('/');await page.getByRole('button',{name:'配置模型',exact:true}).click();const settings=page.getByRole('dialog');await readable(settings);
 await settings.getByLabel('API 密钥').fill('fixture-key');await settings.getByRole('button',{name:'关闭',exact:true}).click();await readable(page.getByRole('alertdialog'));await page.getByRole('button',{name:'取消',exact:true}).click();
 await page.goto('/#/dictionary');await page.reload();await page.getByRole('button',{name:'添加词语'}).click();await readable(page.getByRole('dialog'));await page.getByLabel('词语',{exact:true}).fill('可读性检查');await page.getByRole('button',{name:'保存',exact:true}).click();
 await page.getByRole('button',{name:'更多操作 可读性检查'}).click();await page.getByRole('menuitem',{name:'删除',exact:true}).click();await readable(page.getByRole('dialog'));await page.screenshot({path:`tests/visual/popup-delete-${size.width}.png`});
 await context.close();
});
