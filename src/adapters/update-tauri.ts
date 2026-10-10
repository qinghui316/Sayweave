import {invoke,isTauri} from '@tauri-apps/api/core';
import {listen,type UnlistenFn} from '@tauri-apps/api/event';
import {initialUpdate,type UpdatePort,type UpdateSnapshot} from '../contracts/update';
class BrowserUpdate implements UpdatePort {
 snapshot=()=>initialUpdate;
 subscribe=()=>()=>{};
 async check(){} async cancel(){} async install(){} async restart(){}
}
export class NativeUpdate implements UpdatePort {
 private value:UpdateSnapshot=initialUpdate;
 private listeners=new Set<()=>void>();
 private release:UnlistenFn|null=null;
 private generation=0;
 snapshot=()=>this.value;
 subscribe=(listener:()=>void)=>{
  this.listeners.add(listener);
  if(this.listeners.size===1)void this.connect();
  return()=>{this.listeners.delete(listener);if(!this.listeners.size){this.generation++;this.release?.();this.release=null;}};
 };
 private accept=(value:UpdateSnapshot)=>{
  if(value.revision<this.value.revision)return;
  this.value=Object.freeze(value);for(const listener of this.listeners)listener();
 };
 private async connect(){
  const generation=++this.generation;
  try{
   const release=await listen<UpdateSnapshot>('update:snapshot',event=>{if(generation===this.generation)this.accept(event.payload);});
   if(generation!==this.generation){release();return;}this.release=release;
   const value=await invoke<UpdateSnapshot>('update_snapshot');
   if(generation===this.generation)this.accept(value);
  }catch{if(generation===this.generation){this.release?.();this.release=null;this.value=Object.freeze({...this.value,error:'无法读取更新状态，请重新打开页面。'});for(const listener of this.listeners)listener();}}
 }
 private async command(command:string,args?:Record<string,unknown>){this.accept(await invoke<UpdateSnapshot>(command,args));}
 check=(manual=false)=>this.command('update_check',{manual});
 cancel=()=>this.command('update_cancel');
 install=(taskId:string,revision:number)=>this.command('update_install',{taskId,revision});
 restart=()=>invoke<void>('update_restart');
}
export function createUpdatePort():UpdatePort{return isTauri()?new NativeUpdate():new BrowserUpdate();}
