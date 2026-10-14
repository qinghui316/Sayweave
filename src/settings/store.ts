// Main-window owned projection. Request lifetime does not depend on a mounted page.
export type SettingPatch<T> = Partial<Omit<T,'revision'>>;
export interface SettingState<T> {value:T|null;ready:boolean;busy:boolean;error:string|null}
export class SettingStore<T extends {revision:number}> {
 private confirmed:T|null=null;
 private pending:SettingPatch<T>={};
 private flight:SettingPatch<T>|null=null;
 private uncertain:T|null=null;
 private running=false;
 private paused=false;
 private reading:Promise<void>|null=null;
 private writeEpoch=0;
 private error:string|null=null;
 private listeners=new Set<()=>void>();
 private state:SettingState<T>={value:null,ready:false,busy:false,error:null};
 constructor(private read:()=>Promise<T>,private save:(value:T)=>Promise<T>,private saveError:(error:unknown)=>string=()=> '保存失败，请重试。'){}
 subscribe=(listener:()=>void)=>{this.listeners.add(listener);return()=>{this.listeners.delete(listener);};};
 getSnapshot=()=>this.state;
 getConfirmed=()=>this.confirmed;
 private emit(){
  this.state={value:this.uncertain?{...this.uncertain,...this.pending}:(this.confirmed?{...this.confirmed,...this.flight,...this.pending}:null),ready:!!this.confirmed&&!this.uncertain&&!this.paused,busy:this.running,error:this.error};
  for(const listener of this.listeners)listener();
 }
 accept(value:T){if(!this.confirmed||value.revision>=this.confirmed.revision){this.confirmed=value;this.emit();}}
 load=():Promise<void>=>{
  if(this.reading)return this.reading;
  const epoch=this.writeEpoch;
  this.reading=this.read().then(value=>{
   if(!this.confirmed||value.revision>=this.confirmed.revision)this.confirmed=value;
   if(epoch===this.writeEpoch){this.uncertain=null;this.error=null;}
  }).catch(()=>{if(epoch===this.writeEpoch)this.error='无法读取设置，请重试。';}).finally(()=>{this.reading=null;this.emit();if(this.confirmed&&!this.uncertain&&Object.keys(this.pending).length)void this.drain();});
  return this.reading;
 };
 update=(patch:SettingPatch<T>)=>{
  if(this.paused)return;
  if(!this.confirmed||this.uncertain)return;
  this.pending={...this.pending,...patch};this.error=null;
  void this.drain();
 };
 pause(){this.paused=true;this.emit();}
 resume(){this.paused=false;this.emit();}
 async flush(){
  if(this.reading)await this.reading;
  if(!this.confirmed)await this.load();
  while(this.running)await new Promise(resolve=>setTimeout(resolve,10));
  if(this.uncertain)await this.load();
  if(!this.confirmed||this.uncertain||this.error||Object.keys(this.pending).length)throw new Error(this.error??'设置尚未完成保存。');
 }
 private matches(value:T,patch:SettingPatch<T>){return (Object.keys(patch) as (keyof SettingPatch<T>)[]).every(key=>Object.is(value[key],patch[key]));}
 private async drain(){
  if(this.running){this.emit();return;}
  this.running=true;
  try{
   while(this.confirmed&&!this.uncertain&&Object.keys(this.pending).length){
    const patch=this.pending;this.pending={};
    if(this.matches(this.confirmed,patch)){this.error=null;this.emit();continue;}
    this.flight=patch;
    const requested={...this.confirmed,...patch};this.emit();
    try{
     const saved=await this.save(requested);
     this.writeEpoch++;
     if(saved.revision>=this.confirmed.revision)this.confirmed=saved;
     this.error=null;
    }catch(error){
     // A failed IPC response does not establish whether the database committed.
     // Reconcile by reading once; never blindly resend the failed operation.
     this.writeEpoch++;
     try{
      const current=await this.read();
      if(current.revision>=this.confirmed.revision)this.confirmed=current;
      this.error=this.matches(this.confirmed,patch)?null:this.saveError(error);
     }catch{
      this.uncertain=requested;
      this.error='暂时无法确认设置，请重新读取。';
     }
    }finally{this.flight=null;this.emit();}
   }
  }finally{this.running=false;this.emit();}
 }
}
