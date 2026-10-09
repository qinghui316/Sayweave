import {isTauri} from '@tauri-apps/api/core';
import {check,type Update,type DownloadEvent} from '@tauri-apps/plugin-updater';
import type {UpdatePort,UpdateSnapshot} from '../contracts/update';
const idle:UpdateSnapshot={state:'idle',version:null,progress:null,error:null};
class BrowserUpdate implements UpdatePort { snapshot(){return idle;} subscribe(){return()=>{};} async check(){return;} async install(){return;} setInstallAllowed(){} }
class NativeUpdate implements UpdatePort {
 private value:UpdateSnapshot={...idle}; private listeners=new Set<()=>void>(); private update:Update|null=null; private pending:Promise<void>|null=null; private allowed=true;
 snapshot(){return this.value;} subscribe(listener:()=>void){this.listeners.add(listener);return()=>this.listeners.delete(listener);} setInstallAllowed(value:boolean){this.allowed=value;}
 check(manual=false){if(this.pending)return this.pending;if(!import.meta.env.PROD&&!manual)return Promise.resolve();this.pending=this.runCheck().finally(()=>{this.pending=null;});return this.pending;}
 async install(){if(!this.allowed||this.value.state!=='readyToInstall'||!this.update)return;this.set({state:'installing',progress:100,error:null});try{await this.update.install({restartAfterInstall:true});}catch{this.set({state:'failed',progress:null,error:'更新安装失败'});}}
 private async runCheck(){this.set({state:'checking',progress:null,error:null});try{const update=await check({timeout:15000});if(!update){this.update=null;this.set({...idle});return;}this.update=update;this.set({state:'available',version:update.version,progress:null,error:null});await this.download(update);}catch{this.set({state:'failed',progress:null,error:'暂时无法检查更新'});}}
 private async download(update:Update){let total=0,received=0;this.set({state:'downloading',version:update.version,progress:0,error:null});try{await update.download((event:DownloadEvent)=>{if(event.event==='Started')total=event.data.contentLength??0;if(event.event==='Progress')received+=event.data.chunkLength;this.set({state:'downloading',version:update.version,progress:total?Math.min(100,Math.floor(received/total*100)):null,error:null});});this.set({state:'readyToInstall',version:update.version,progress:100,error:null});}catch{this.set({state:'failed',version:update.version,progress:null,error:'更新下载失败'});}}
 private set(next:Partial<UpdateSnapshot>){this.value=Object.freeze({...this.value,...next});this.listeners.forEach(listener=>listener());}
}
export function createUpdatePort():UpdatePort{if(!isTauri())return new BrowserUpdate();const port=new NativeUpdate();if(import.meta.env.PROD)setTimeout(()=>void port.check(),20000);return port;}
