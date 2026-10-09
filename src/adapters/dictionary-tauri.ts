import { invoke, isTauri } from '@tauri-apps/api/core';
import { dictionaryError, parseDictionaryPage, parseDictionaryRecord, parseDictionaryStatus, type DictionaryPort, type DictionaryInput, type DictionaryQuery, type DictionaryKind } from '../contracts/dictionary';
import { BrowserDictionary } from './dictionary-browser';
import {listen} from '@tauri-apps/api/event';
class TauriDictionary implements DictionaryPort {
  readonly persistence='disk' as const;
  async onChanged(callback:()=>void){return listen('dictionary:changed',callback);}
  private async call(command: string,args: Record<string,unknown>) { try { return await invoke<unknown>(command,args); } catch(e) { throw dictionaryError(e); } }
  async status(retry=false) { return parseDictionaryStatus(await this.call('dictionary_status',{retry})); }
  async list(query: DictionaryQuery) { return parseDictionaryPage(await this.call('dictionary_list',{query})); }
  async get(kind: DictionaryKind,id: string) { return parseDictionaryRecord(await this.call('dictionary_get',{kind,id})); }
  async create(input: DictionaryInput) { return parseDictionaryRecord(await this.call('dictionary_create',{input})); }
  async update(input: DictionaryInput,id: string,revision: number) { return parseDictionaryRecord(await this.call('dictionary_update',{input,id,revision})); }
  async delete(kind: DictionaryKind,id: string,revision: number) { await this.call('dictionary_delete',{kind,id,revision}); }
}
export function createDictionary(): DictionaryPort { return isTauri()?new TauriDictionary():new BrowserDictionary(); }
