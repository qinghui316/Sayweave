import {useEffect,useSyncExternalStore} from 'react';
import type {SettingStore} from './store';
export function useSetting<T extends {revision:number}>(store:SettingStore<T>){
 const state=useSyncExternalStore(store.subscribe,store.getSnapshot);
 useEffect(()=>{void store.load();},[store]);
 return state;
}
