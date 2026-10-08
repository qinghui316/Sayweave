import {speechError} from '../../contracts/speech';
import type {HistoryCursor,HistoryPort,HistorySummary} from '../../contracts/history';
export interface ReadingState {search:string;records:HistorySummary[];total:number;cursor:HistoryCursor|null;page:number;epoch:number;loaded:boolean;loading:boolean;loadingMore:boolean;error:string|null;newRecords:boolean;expandedId:string|null;scrollTop:number}
interface Bookmark {ids:string[];start:HistoryCursor|null;end:HistoryCursor|null;next:HistoryCursor|null;scrollTop:number}
const initial=():ReadingState=>({search:'',records:[],total:0,cursor:null,page:1,epoch:0,loaded:false,loading:false,loadingMore:false,error:null,newRecords:false,expandedId:null,scrollTop:0});
/** Reading boundaries are immutable until explicit refresh/search. Payload cache is bounded. */
export class HistoryFeedStore {
 private value=initial();private listeners=new Set<()=>void>();private generation=0;
 private timer:ReturnType<typeof setTimeout>|undefined;private scrollTop=0;
 private pages:Bookmark[]=[];private cache=new Map<number,HistorySummary[]>();private retryPage:number|null=null;private refreshPending=false;
 constructor(private readonly port:HistoryPort){}
 getSnapshot=()=>this.value;
 subscribe=(listener:()=>void)=>{this.listeners.add(listener);return()=>{this.listeners.delete(listener);};};
 private set(patch:Partial<ReadingState>){this.value={...this.value,...patch};for(const fn of this.listeners)fn();}
 private cachePage(index:number,records:HistorySummary[]){this.cache.delete(index);this.cache.set(index,records);while(this.cache.size>3)this.cache.delete(this.cache.keys().next().value!);}
 setScroll(top:number){this.scrollTop=top;const page=this.pages[this.value.page-1];if(page)page.scrollTop=top;}
 getScroll=()=>this.scrollTop;
 expand(id:string|null){if(!this.value.loadingMore)this.set({expandedId:id});}
 private reset(search:string){clearTimeout(this.timer);this.generation++;this.pages=[];this.cache.clear();this.retryPage=null;this.refreshPending=false;this.scrollTop=0;this.set({...initial(),search,epoch:this.value.epoch+1,loading:true});}
 search(text:string){this.reset(text);this.timer=setTimeout(()=>void this.refresh(),text?180:0);}
 async latest(){this.reset(this.value.search);await this.refresh(undefined,true);}
 private async ready(retry=false){const status=await this.port.status(retry);if(status.state!=='ready')throw speechError({code:status.state==='preparing'?'preparing':'unavailable'});}
 async refresh(_atTop?:boolean,retry=false){
  clearTimeout(this.timer);if(this.value.loadingMore){this.refreshPending=true;return;}
  const token=++this.generation,prior=this.value,index=prior.page-1;
  this.set({loading:!prior.loaded,error:null});
  try{
   await this.ready(retry);if(token!==this.generation)return;
   const head=await this.port.browse(prior.search,null,20);if(token!==this.generation)return;
   if(!prior.loaded){
    const end=head.records.at(-1);this.pages=[{ids:head.records.map(r=>r.id),start:null,end:end?{id:end.id,createdAt:end.createdAt}:null,next:head.nextCursor,scrollTop:0}];
    this.cachePage(0,head.records);this.set({records:head.records,total:head.total,cursor:head.nextCursor,loaded:true,loading:false});return;
   }
   const bookmark=this.pages[index]!;
   const page=await this.port.reread(prior.search,bookmark.ids);if(token!==this.generation)return;
   const first=this.pages[0];
   const incoming=head.records.some(r=>!first?.end||r.createdAt>first.end.createdAt||r.createdAt===first.end.createdAt&&r.id<=first.end.id ? !first?.ids.includes(r.id) : false);
   this.cache.clear();this.acceptPage(index,page.records);this.set({total:page.total,loading:false,newRecords:prior.newRecords||incoming});
   await this.leaveEmpty();
  }catch(e){if(token===this.generation)this.set({loading:false,error:speechError(e).code});}
 }
 private nextIndex(index=this.value.page-1){
  let next=index+1;
  while(this.pages[next]&&this.pages[next]!.ids.length===0)next++;
  return this.pages[next]||this.pages[next-1]?.next?next:null;
 }
 private acceptPage(index:number,records:HistorySummary[]){
  const bookmark=this.pages[index]!;const byId=new Map(records.map(r=>[r.id,r]));
  bookmark.ids=bookmark.ids.filter(id=>byId.has(id));const ordered=bookmark.ids.map(id=>byId.get(id)!);
  this.cachePage(index,ordered);
  this.set({records:ordered,page:index+1,cursor:this.nextIndex(index)===null?null:bookmark.next,expandedId:ordered.some(r=>r.id===this.value.expandedId)?this.value.expandedId:null});
 }
 async go(index:number,retry=false){
  if(this.value.loading||this.value.loadingMore||index<0||index>this.pages.length||index===this.value.page-1)return;
  const known=this.pages[index],previous=this.pages[index-1];if(!known&&!previous?.next)return;
  const token=++this.generation,search=this.value.search;this.retryPage=index;
  this.set({loadingMore:true,error:null,epoch:this.value.epoch+1});
  try{
   if(retry)await this.ready(true);
   const cached=this.cache.get(index);
   const result=known?(cached?{records:cached,total:this.value.total}:await this.port.reread(search,known.ids)):await this.port.browse(search,previous!.next,20);
   if(token!==this.generation)return;
   if(!known){const end=result.records.at(-1);this.pages[index]={ids:result.records.map(r=>r.id),start:previous!.next,end:end?{id:end.id,createdAt:end.createdAt}:previous!.end??null,next:'nextCursor'in result?result.nextCursor:null,scrollTop:0};}
   this.scrollTop=0;this.set({scrollTop:0,expandedId:null,total:result.total,loadingMore:false});this.acceptPage(index,result.records);this.retryPage=null;
   await this.leaveEmpty();
  }catch(e){if(token===this.generation)this.set({loadingMore:false,error:speechError(e).code});}
  finally{if(token===this.generation&&this.refreshPending){
   this.refreshPending=false;const error=this.value.error,expected=this.generation+1;await this.refresh();
   if(error&&this.generation===expected)this.set({error});
  }}
 }
 next=()=>{const index=this.nextIndex();return index===null?Promise.resolve():this.go(index);};
 previous=()=>this.go(this.value.page-2);
 retry=()=>this.retryPage===null?this.refresh(undefined,true):this.go(this.retryPage,true);
 private async leaveEmpty(){
  if(this.value.records.length)return;
  // Prefer the prior nonempty reading page, otherwise seek forward past empty boundaries.
  let previous=this.value.page-2;
  while(previous>=0&&this.pages[previous]?.ids.length===0)previous--;
  if(previous>=0){await this.go(previous);return;}
  const next=this.nextIndex();if(next!==null)await this.go(next);
 }
 async update(id:string){if(!this.pages.some(page=>page.ids.includes(id)))this.set({newRecords:true});await this.refresh();}
 remove(id:string){
  this.generation++;this.retryPage=null;
  for(const page of this.pages)page.ids=page.ids.filter(item=>item!==id);
  for(const [index,records]of [...this.cache])this.cachePage(index,records.filter(r=>r.id!==id));
  this.set({loading:false,loadingMore:false,epoch:this.value.epoch+1,records:this.value.records.filter(r=>r.id!==id),expandedId:this.value.expandedId===id?null:this.value.expandedId});
  // Count comes from a successful query, never a duplicate delete notification.
  void this.refresh();
 }
}
