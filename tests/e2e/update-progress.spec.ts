import {test,expect} from '@playwright/test';
// Render the production App and SettingsShell. Only the native update snapshot is controlled.
const updateAdapter=`import {initialUpdate} from '/src/contracts/update.ts';
let value={...initialUpdate,build:{...initialUpdate.build,version:'0.1.11',channel:'stable',updatesEnabled:true}};
const listeners=new Set();
window.addEventListener('fixture:update',event=>{value={...value,...event.detail};listeners.forEach(fn=>fn());});
export function createUpdatePort(){return {snapshot:()=>value,subscribe:fn=>{listeners.add(fn);return()=>listeners.delete(fn);},check:async()=>{},cancel:async()=>{},install:async()=>{},restart:async()=>{}};}`;
for(const size of [{width:1120,height:760,scale:1,motion:'no-preference' as const},{width:960,height:640,scale:1.5,motion:'reduce' as const}])test(`sidebar update and original settings ${size.width}`,async({browser})=>{
 const context=await browser.newContext({viewport:size,deviceScaleFactor:size.scale,reducedMotion:size.motion}),page=await context.newPage();
 await page.route('**/src/adapters/update-tauri.ts*',route=>route.fulfill({contentType:'application/javascript',body:updateAdapter}));
 await page.goto('/');await expect(page.getByRole('button',{name:'关于',exact:true})).toBeVisible();
 for(const [state,progress,visual] of [['checking',null,'indeterminate'],['downloading',null,'indeterminate'],['downloading',0,'progress'],['downloading',43,'progress'],['downloading',100,'progress'],['verifying',100,'indeterminate'],['readyToInstall',100,'download'],['preparing',null,'indeterminate'],['failed',null,'error'],['recovery',null,'error']] as const){
  await page.evaluate(value=>window.dispatchEvent(new CustomEvent('fixture:update',{detail:value})),{state,progress,version:'0.1.11',taskId:'fixture',recovery:state==='recovery',error:state==='failed'||state==='recovery'?'更新未完成，请重试。':null});
  const trigger=page.getByRole('button',{name:/打开关于$/});await expect(trigger.locator('svg')).toHaveAttribute('data-update-visual',visual);
  if(visual==='indeterminate'&&size.motion==='reduce')await expect(trigger.locator('circle').last()).toHaveCSS('animation-name','none');
  await trigger.focus();await expect(trigger).toBeFocused();await page.keyboard.press('Enter');
  const settings=page.getByRole('dialog',{name:'设置',exact:true});await expect(settings).toBeVisible();
  await expect(settings.getByRole('button',{name:'关于',exact:true})).toHaveAttribute('aria-current','page');
  await expect(settings.locator('[data-update-visual]')).toHaveCount(0);
  if(state==='readyToInstall')await expect(settings.getByRole('button',{name:'重启并更新',exact:true})).toBeEnabled();
  if(state==='downloading'){if(progress!==null)await expect(settings.getByRole('progressbar')).toHaveAttribute('aria-valuenow',String(progress));else await expect(settings.getByRole('progressbar')).not.toHaveAttribute('aria-valuenow');}
  await page.evaluate(()=>document.fonts.ready);
  if(state==='readyToInstall')await page.screenshot({path:`tests/visual/update-about-original-${size.width}.png`});
  await settings.getByRole('button',{name:'关闭',exact:true}).click();await expect(settings).not.toBeVisible();
  await trigger.evaluate(el=>(el as HTMLElement).blur());
  if(['downloading','readyToInstall'].includes(state))await page.screenshot({path:`tests/visual/update-main-${state}-${progress??'unknown'}-${size.width}.png`});
 }
 await page.emulateMedia({forcedColors:'active'});await expect(page.locator('[data-update-visual]')).toBeVisible();await context.close();
});
