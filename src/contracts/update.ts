export type UpdateState = 'idle'|'checking'|'available'|'downloading'|'readyToInstall'|'installing'|'failed';
export type UpdateSnapshot = Readonly<{state:UpdateState;version:string|null;progress:number|null;error:string|null}>;
export interface UpdatePort { snapshot():UpdateSnapshot; subscribe(listener:()=>void):()=>void; check(manual?:boolean):Promise<void>; install():Promise<void>; setInstallAllowed(allowed:boolean):void; }
