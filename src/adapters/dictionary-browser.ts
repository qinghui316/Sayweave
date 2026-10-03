import { DictionaryError, dictionaryKey, validateDictionaryInput, type DictionaryPort, type DictionaryInput, type DictionaryQuery, type DictionaryKind, type DictionaryRecord } from '../contracts/dictionary';
export class BrowserDictionary implements DictionaryPort {
  readonly persistence='memory' as const;
  private records: DictionaryRecord[]=[];
  async status() { return {state:'ready' as const,error:null,schemaVersion:1,normalizationVersion:1}; }
  async list(query: DictionaryQuery) {
    if(!Number.isInteger(query.page)||query.page<0||[...query.search].length>128||/\p{Cc}/u.test(query.search)) throw new DictionaryError('invalid');
    const key=dictionaryKey(query.search);
    const records=this.records.filter(r=>r.kind===query.kind && [r.original,r.category,r.replacement].some(s=>s!==null && dictionaryKey(s).includes(key))).sort((a,b)=>b.updatedAt-a.updatedAt || (a.id<b.id?-1:a.id>b.id?1:0));
    const page=Math.min(Math.max(1,query.page),Math.max(1,Math.ceil(records.length/50)));
    return {records:records.slice((page-1)*50,page*50).map(r=>({...r})),total:records.length,page};
  }
  private save(input: DictionaryInput, existing?: DictionaryRecord) {
    const clean=validateDictionaryInput(input);
    const duplicate=this.records.find(r=>r.kind===clean.kind && r.id!==existing?.id && dictionaryKey(r.original)===dictionaryKey(clean.original));
    if(duplicate) throw new DictionaryError('duplicate',duplicate.id);
    const now=Date.now();
    const record: DictionaryRecord={...clean,id:existing?.id??crypto.randomUUID(),revision:(existing?.revision??0)+1,createdAt:existing?.createdAt??now,updatedAt:now};
    this.records=this.records.filter(r=>r.id!==record.id).concat(record); return {...record};
  }
  async create(input: DictionaryInput) { return this.save(input); }
  async get(kind: DictionaryKind,id: string) { const record=this.records.find(r=>r.kind===kind&&r.id===id); if(!record)throw new DictionaryError('notFound'); return {...record}; }
  async update(input: DictionaryInput,id: string,revision: number) { return this.save(input,this.current(input.kind,id,revision)); }
  private current(kind: DictionaryKind,id: string,revision: number) {
    const record=this.records.find(r=>r.id===id&&r.kind===kind); if(!record) throw new DictionaryError('notFound'); if(record.revision!==revision) throw new DictionaryError('conflict'); return record;
  }
  async delete(kind: DictionaryKind,id: string,revision: number) { this.current(kind,id,revision); this.records=this.records.filter(r=>r.id!==id); }
}
