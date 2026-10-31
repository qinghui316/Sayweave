import {test,expect} from '@playwright/test';
import type {HistoryRecord} from '../../src/contracts/history';
const records:HistoryRecord[]=Array.from({length:1000},(_,n)=>{const id=`history-${String(n).padStart(4,'0')}`,at=Date.now()-n*60000;return {id,createdAt:at,endedAt:at+1000,durationMs:1000,status:'success',currentAttempt:`attempt-${n}`,revision:1,audio:null,attempts:[{id:`attempt-${n}`,recordId:id,kind:'live',configId:'fixture',configName:'模型',configRevision:1,modelId:'fixture',serviceHost:'example.invalid',createdAt:at,endedAt:at+1000,status:'success',text:n===0?'长中文识别文字，支持选中、完整复制和在记录下展开。'.repeat(50):`记录 ${n}：今天的口述文字。`,errorCode:null,elapsedMs:1}]};});
test('history follows archive notifications automatically and preserves active reading geometry',async({page})=>{
 await page.route('**/src/adapters/speech-browser.ts',async route=>{
  const response=await route.fetch();let source=await response.text();expect(source).toContain('records = [];');
  source=source.replace('records = [];',`records = ${JSON.stringify(records.slice(0,45))};`);
  source+=`\nconst originalSubscribe = BrowserSpeech.prototype.subscribe;
BrowserSpeech.prototype.subscribe = async function(...args) {
 const release = await originalSubscribe.apply(this,args);
 const archive = event => { const r = event.detail; const index = this.records.findIndex(row => row.id === r.id); if(index<0)this.records.push(r);else this.records[index]=r;window.dispatchEvent(new Event('sayweave-test-archived')); };
 window.addEventListener('sayweave-test-archive',archive);
 window.historyFixtureSpeechReady = true;
 return () => {release();window.removeEventListener('sayweave-test-archive',archive);};
};`;
  await route.fulfill({response,body:source});
 });
 await page.route('**/src/adapters/processing-tauri.ts',async route=>{
  const response=await route.fetch();const source=await response.text();
  await route.fulfill({response,body:source+`\nBrowserProcessing.prototype.onArchived = async function(callback) { const listener=()=>callback();window.addEventListener('sayweave-test-archived',listener);window.historyFixtureArchiveReady = true;return () => window.removeEventListener('sayweave-test-archived',listener); };`});
 });
 const archive=async(n:number,text:string)=>{
  const id=`new-history-${n}`,at=records[0]!.createdAt+n*1000,r:HistoryRecord={...records[1]!,id,createdAt:at,currentAttempt:`new-attempt-${n}`,attempts:[{...records[1]!.attempts[0]!,id:`new-attempt-${n}`,recordId:id,text}]};
  await page.evaluate(record=>window.dispatchEvent(new CustomEvent('sayweave-test-archive',{detail:record})),r);return r;
 };
 await page.goto('/#/history');await expect(page.getByText('45 条',{exact:true})).toBeVisible();
 await expect.poll(()=>page.evaluate(()=>({speech:Reflect.get(window,'historyFixtureSpeechReady'),archive:Reflect.get(window,'historyFixtureArchiveReady')}))).toEqual({speech:true,archive:true});
 const list=page.getByLabel('历史记录列表',{exact:true}),rows=page.getByRole('listitem');
 await archive(1,'第一轮刚保存的口述');await expect(page.getByText('46 条',{exact:true})).toBeVisible();await expect(rows.first()).toHaveAttribute('data-record-id','new-history-1');
 await expect(page.getByRole('button',{name:/查看最新/})).toHaveCount(0);expect(await list.evaluate(node=>node.scrollTop)).toBe(0);
 await list.evaluate(node=>{node.scrollTop=200;node.dispatchEvent(new Event('scroll'));});
 const ids=await rows.evaluateAll(nodes=>nodes.map(node=>node.getAttribute('data-record-id'))),top=await list.evaluate(node=>node.scrollTop);
 await archive(2,'阅读期间完成的口述');await expect(page.getByText('47 条',{exact:true})).toBeVisible();
 expect(await rows.evaluateAll(nodes=>nodes.map(node=>node.getAttribute('data-record-id')))).toEqual(ids);expect(await list.evaluate(node=>node.scrollTop)).toBe(top);
 await list.evaluate(node=>{node.scrollTop=0;node.dispatchEvent(new Event('scroll'));});await expect(rows.first()).toHaveAttribute('data-record-id','new-history-2');expect(await list.evaluate(node=>node.scrollTop)).toBe(0);
 await rows.first().getByRole('button',{name:'更多操作'}).focus();await page.keyboard.press('Enter');await page.getByRole('menuitem',{name:'识别版本'}).click();await expect(rows.first().getByRole('region',{name:'历史识别文字'})).toBeVisible();
 const latest=await archive(3,'展开期间完成的口述');await expect(page.getByText('48 条',{exact:true})).toBeVisible();await expect(rows.first()).toHaveAttribute('data-record-id','new-history-2');
 expect(await list.evaluate(node=>node.scrollTop),'reading position before collapsing').toBe(0);
 await rows.first().getByRole('button',{name:'收起记录'}).click();
 await expect(rows.first()).toHaveAttribute('data-record-id','new-history-3');
 latest.revision++;latest.processing=[{id:'processed-new',attemptId:latest.currentAttempt!,configName:'文字模型',modelId:'fixture',state:'success',text:'稍后保存的完整译文',revision:1,errorCode:null,units:[]}];
 await page.evaluate(record=>window.dispatchEvent(new CustomEvent('sayweave-test-archive',{detail:record})),latest);await expect(rows.first()).toContainText('稍后保存的完整译文');
 await page.getByRole('button',{name:'下一页'}).click();await expect(page.getByLabel('历史分页')).toContainText('第 2 页');const older=await rows.evaluateAll(nodes=>nodes.map(node=>node.getAttribute('data-record-id')));
 await archive(4,'翻页期间完成的口述');await expect(page.getByText('49 条',{exact:true})).toBeVisible();expect(await rows.evaluateAll(nodes=>nodes.map(node=>node.getAttribute('data-record-id')))).toEqual(older);
 await page.getByRole('button',{name:'上一页'}).click();await expect(rows.first()).toHaveAttribute('data-record-id','new-history-4');
 await page.screenshot({path:'.local/history-auto-refresh/history-auto.png'});
});
test('history keeps 20-row pages, reading position and accessible controls',async({page})=>{
 test.setTimeout(60000);await page.setViewportSize({width:1120,height:760});
 await page.route('**/src/adapters/speech-browser.ts',async route=>{const response=await route.fetch();const source=await response.text();expect(source).toContain('records = [];');await route.fulfill({response,body:source.replace('records = [];',`records = ${JSON.stringify(records)};`)});});
 await page.goto('/#/history');await expect(page.getByText('1000 条',{exact:true})).toBeVisible();
 const first=page.getByRole('listitem').first(),text=first.locator('p').first(),before=await text.boundingBox();await first.hover();await expect(first.getByRole('button',{name:'复制',exact:true})).toBeVisible();expect(await text.boundingBox()).toEqual(before);
 await first.getByRole('button',{name:'展开',exact:true}).click();await expect(first.getByRole('region',{name:'历史识别文字'})).toContainText(records[0]!.attempts[0]!.text);await expect(page.getByRole('dialog')).toHaveCount(0);await first.getByRole('button',{name:'收起记录'}).click();
 const scroller=page.getByLabel('历史记录列表',{exact:true});
 await expect(page.getByRole('listitem')).toHaveCount(20);
 const ids=new Set<string>();
 for(let batch=0;batch<4;batch++){
  const current=await page.getByRole('listitem').evaluateAll(rows=>rows.map(row=>row.getAttribute('data-record-id')!));
  for(const id of current){expect(ids.has(id)).toBe(false);ids.add(id);}
  await page.getByRole('button',{name:'下一页'}).click();await expect(page.getByLabel('历史分页')).toContainText(`第 ${batch+2} 页`);
 }
 await expect(page.getByRole('listitem')).toHaveCount(20);
 await scroller.evaluate(node=>{node.scrollTop=300;});const top=await scroller.evaluate(node=>node.scrollTop);
 await page.getByRole('link',{name:'首页',exact:true}).click();await page.getByRole('link',{name:'历史记录',exact:true}).click();await expect(page.getByRole('listitem')).toHaveCount(20);await expect(page.getByLabel('历史分页')).toContainText('第 5 页');await expect.poll(()=>scroller.evaluate(node=>node.scrollTop)).toBe(top);
 await page.getByRole('button',{name:'上一页'}).click();await expect(page.getByLabel('历史分页')).toContainText('第 4 页');
 await expect(page.getByRole('listitem').first()).toHaveAttribute('data-record-id','history-0060');
 await page.getByRole('button',{name:'搜索识别文字'}).click();await page.getByRole('textbox',{name:'搜索识别文字'}).fill('记录 9');await expect(page.getByText('111 条',{exact:true})).toBeVisible();
 await page.getByRole('button',{name:'历史保存设置',exact:true}).click();await page.getByRole('combobox',{name:'录音保留时间'}).click();await page.getByRole('option',{name:'7 天',exact:true}).click();await page.getByRole('link',{name:'词典',exact:true}).click();await expect(page.getByRole('alertdialog')).toHaveCount(0);await page.getByRole('link',{name:'历史记录',exact:true}).click();await expect(page.getByRole('textbox',{name:'搜索识别文字'})).toHaveValue('记录 9');
 await page.setViewportSize({width:960,height:640});const row=page.getByRole('listitem').first();await row.getByRole('button',{name:'更多操作'}).focus();await expect(row.getByRole('button',{name:'复制',exact:true})).toBeVisible();await page.keyboard.press('Enter');await expect(page.getByRole('menuitem',{name:'删除记录'})).toBeVisible();await page.keyboard.press('Escape');await expect(row.getByRole('button',{name:'更多操作'})).toBeFocused();
 expect(await page.locator('main').evaluate(node=>node.scrollHeight<=node.clientHeight+1)).toBe(true);expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
 await scroller.evaluate(node=>{node.scrollTop=node.scrollHeight;});
 const last=page.getByRole('listitem').last();await last.scrollIntoViewIfNeeded();
 const bottom=(await last.boundingBox())!,list=(await scroller.boundingBox())!,footer=(await page.getByLabel('历史分页').boundingBox())!;
 expect(bottom.y+bottom.height).toBeLessThanOrEqual(list.y+list.height+1);expect(list.y+list.height).toBeLessThanOrEqual(footer.y);
 await page.screenshot({path:'.local/settlement-ui/history-960.png'});

});
