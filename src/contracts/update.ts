export type UpdateState = 'idle'|'checking'|'downloading'|'verifying'|'readyToInstall'|'preparing'|'stopping'|'installing'|'recovery'|'failed';
export type BuildInfo = Readonly<{version:string;commit:string;channel:string;target:string;updatesEnabled:boolean}>;
export type UpdateSnapshot = Readonly<{revision:number;taskId:string|null;state:UpdateState;version:string|null;progress:number|null;error:string|null;failureStage:string|null;blockedReason:string|null;recovery:boolean;notes:string|null;build:BuildInfo}>;
export interface UpdatePort {
 snapshot():UpdateSnapshot;
 subscribe(listener:()=>void):()=>void;
 check(manual?:boolean):Promise<void>;
 cancel():Promise<void>;
 install(taskId:string,revision:number):Promise<void>;
 restart():Promise<void>;
}
export const initialUpdate:UpdateSnapshot=Object.freeze({revision:0,taskId:null,state:'idle',version:null,progress:null,error:null,failureStage:null,blockedReason:null,recovery:false,notes:null,build:{version:'',commit:'',channel:'internal',target:'',updatesEnabled:false}});
