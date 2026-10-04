import {speechError} from '../../contracts/speech';
import {summarizeHistory,type HistoryCursor,type HistoryPort,type HistorySummary} from '../../contracts/history';
export interface ReadingState {search:string;records:HistorySummary[];total:number;cursor:HistoryCursor|null;loaded:boolean;loading:boolean;loadingMore:boolean;error:string|null;newRecords:boolean;expandedId:string|null;scrollTop:number}
const sort=(a:HistorySummary,b:HistorySummary)=>b.createdAt-a.createdAt||(a.id<b.id?-1:a.id>b.id?1:0);
export class HistoryFeedStore {
 private value:ReadingState={search:'',records:[],total:0,cursor:null,loaded:false,loading:false,loadingMore:false,error:null,newRecords:false,expandedId:null,scrollTop:0};
 private listeners=new Set<()=>void>();private generation=0;private timer:ReturnType<typeof setTimeout>|undefined;private scrollTop=0;
 constructor(private readonly port:HistoryPort){}
 getSnapshot=()=>this.value;
 subscribe=(listener:()=>void)=>{this.listeners.add(listener);return()=>{this.listeners.delete(listener);};};
 private set(patch:Partial<ReadingState>){this.value={...this.value,...patch};for(const fn of this.listeners)fn();}
 setScroll(top:number){this.scrollTop=top;}
 getScroll=()=>this.scrollTop;
 expand(id:string|null){this.set({expandedId:id});}
 search(text:string){clearTimeout(this.timer);this.generation++;this.scrollTop=0;this.set({search:text,records:[],total:0,cursor:null,loaded:false,loading:true,loadingMore:false,error:null,newRecords:false,expandedId:null,scrollTop:0});this.timer=setTimeout(()=>void this.refresh(true),text?180:0);}
 async refresh(atTop=this.scrollTop<32,retry=false){
  clearTimeout(this.timer);const token=++this.generation;const prior=this.value;
  this.set({loading:!prior.loaded,loadingMore:false,error:null});
  try{const status=await this.port.status(retry);if(status.state!=='ready')throw speechError({code:status.state==='preparing'?'preparing':'unavailable'});
   const page=await this.port.browse(prior.search);
   if(token!==this.generation)return;
   const ids=new Set(prior.records.map(r=>r.id));const incoming=page.records.some(r=>!ids.has(r.id));
   const fresh=new Map(page.records.map(r=>[r.id,r]));
   const records=prior.loaded?prior.records.map(r=>fresh.get(r.id)??r):page.records;
   if(prior.loaded&&atTop)for(const r of page.records)if(!ids.has(r.id))records.push(r);
   records.sort(sort);
   const gap=atTop&&incoming&&!page.records.some(r=>ids.has(r.id));
   this.set({records,total:page.total,cursor:prior.loaded&&prior.records.length>=50&&!gap?prior.cursor:page.nextCursor,loaded:true,loading:false,error:null,newRecords:!atTop&&incoming});
  }catch(e){if(token===this.generation)this.set({loading:false,error:speechError(e).code});}
 }
 async more(retry=false){
  if(this.value.loading||this.value.loadingMore||!this.value.cursor)return;
  const token=this.generation;const search=this.value.search;const cursor=this.value.cursor;this.set({loadingMore:true,error:null});
  try{if(retry){const status=await this.port.status(true);if(status.state!=='ready')throw speechError({code:status.state==='preparing'?'preparing':'unavailable'});}const page=await this.port.browse(search,cursor);if(token!==this.generation)return;
   const records=new Map(this.value.records.map(r=>[r.id,r]));for(const r of page.records)records.set(r.id,r);
   this.set({records:[...records.values()].sort(sort),total:page.total,cursor:page.nextCursor,loadingMore:false});
  }catch(e){if(token===this.generation)this.set({loadingMore:false,error:speechError(e).code});}
 }
 async update(id:string){
  const token=++this.generation;const search=this.value.search;this.set({loading:false,loadingMore:false});
  try{const detail=await this.port.detail(id);if(token!==this.generation)return;
   const summary=summarizeHistory(detail,search);const exists=this.value.records.some(r=>r.id===id);
   const records=this.value.records.filter(r=>r.id!==id);if(summary&&(exists||this.scrollTop<32))records.push(summary);
   this.set({records:records.sort(sort),newRecords:this.value.newRecords||!!summary&&!exists&&this.scrollTop>=32});
  }catch(e){if(token===this.generation&&speechError(e).code==='notFound')this.remove(id);}
 }
 remove(id:string){this.generation++;this.set({loading:false,loadingMore:false,records:this.value.records.filter(r=>r.id!==id),total:Math.max(0,this.value.total-1),expandedId:this.value.expandedId===id?null:this.value.expandedId});}
 latest(){this.generation++;this.scrollTop=0;this.set({records:[],cursor:null,loaded:false,newRecords:false,scrollTop:0,expandedId:null});void this.refresh(true);}
}
