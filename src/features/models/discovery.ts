import {useCallback,useEffect,useRef,useState} from 'react';
import {validateModelKey,type ModelConfigPort,type ModelDraft} from '../../contracts/models';

const caches=new WeakMap<ModelConfigPort,Map<string,{at:number;ids:string[]}>>();
const ttl=15*60*1000;
function endpoint(draft:ModelDraft):string|null {
 try {const url=new URL(draft.input.baseUrl.trim());if(url.protocol!=='https:'||url.username||url.password||url.search||url.hash)return null;return url.href.replace(/\/+$/u,'');}catch{return null;}
}
export function useModelDiscovery(port:ModelConfigPort,draft:ModelDraft|null) {
 const [state,setState]=useState<{ids:string[];loading:boolean;error:unknown;loaded:boolean}>({ids:[],loading:false,error:null,loaded:false});
 const current=useRef(draft);current.current=draft;
 const epoch=useRef(0),request=useRef<string|null>(null),pending=useRef(false);
 const cancel=useCallback(()=>{epoch.current++;pending.current=false;setState(s=>({...s,loading:false}));const id=request.current;request.current=null;if(id)void port.cancelRequest(id).catch(()=>{});},[port]);
 const credential=draft?.key.action==='replace'?draft.key.value:null;
 const address=draft?.input.baseUrl,id=draft?.id,revision=draft?.revision;
 useEffect(()=>{cancel();setState({ids:[],loading:false,error:null,loaded:false});return cancel;},[address,credential,id,revision,cancel]);
 const sync=useCallback(async(force=false)=>{
  const value=current.current;if(!value||pending.current)return;
  const url=endpoint(value);if(!url)return;
  let secret='';
  if(value.key.action==='replace'){try{secret=validateModelKey(value.key.value);}catch{return;}}
  else if(!value.id)return;
  pending.current=true;const version=epoch.current;
  setState(s=>({...s,loading:true,error:null}));
  try {
   // Only an opaque digest enters the process cache; no key is retained there.
   const bytes=await crypto.subtle.digest('SHA-256',new TextEncoder().encode(secret));
   const fingerprint=Array.from(new Uint8Array(bytes),v=>v.toString(16).padStart(2,'0')).join('');
   if(version!==epoch.current)return;
   const key=JSON.stringify([url,value.id,value.revision,fingerprint]);
   let cache=caches.get(port);if(!cache){cache=new Map();caches.set(port,cache);}
   const cached=cache.get(key);
   let ids:string[];
   if(!force&&cached&&Date.now()-cached.at<ttl)ids=cached.ids;
   else {
    const requestId=crypto.randomUUID();request.current=requestId;
    ids=await port.fetchModels({...value,input:{...value.input,baseUrl:url}},requestId);
    if(version!==epoch.current)return;
    if(cache.size>=32)cache.clear();cache.set(key,{ids,at:Date.now()});
   }
   if(version===epoch.current)setState({ids,loading:false,error:null,loaded:true});
  }catch(error){if(version===epoch.current)setState(s=>({...s,loading:false,error,loaded:true}));}
  finally{if(version===epoch.current){pending.current=false;request.current=null;}}
 },[port]);
 return {...state,sync,cancel,valid:!!draft&&!!endpoint(draft)&&(draft.key.action==='replace'?!!draft.key.value.trim():!!draft.id)};
}
