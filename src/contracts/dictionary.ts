export type DictionaryKind = 'term' | 'correction';
export type DictionaryErrorCode = 'invalid'|'duplicate'|'notFound'|'conflict'|'busy'|'unavailable'|'forbidden';
export class DictionaryError extends Error {
  constructor(readonly code: DictionaryErrorCode, readonly existingId?: string) { super(code); }
}
export interface DictionaryInput { kind: DictionaryKind; original: string; replacement: string|null; category: string|null }
export interface DictionaryRecord extends DictionaryInput { id: string; revision: number; createdAt: number; updatedAt: number }
export interface DictionaryQuery { kind: DictionaryKind; search: string; page: number }
export interface DictionaryPage { records: DictionaryRecord[]; total: number; page: number }
export interface DictionaryStatus { state: 'preparing'|'ready'|'unavailable'; error: {code: DictionaryErrorCode}|null; schemaVersion: number; normalizationVersion: number }
export interface DictionaryPort {
  readonly persistence: 'disk'|'memory';
  status(retry?: boolean): Promise<DictionaryStatus>;
  list(query: DictionaryQuery): Promise<DictionaryPage>;
  get(kind: DictionaryKind, id: string): Promise<DictionaryRecord>;
  create(input: DictionaryInput): Promise<DictionaryRecord>;
  update(input: DictionaryInput, id: string, revision: number): Promise<DictionaryRecord>;
  delete(kind: DictionaryKind, id: string, revision: number): Promise<void>;
}
export function trimText(value: string) { return value.replace(/^\p{White_Space}+|\p{White_Space}+$/gu, ''); }
export function dictionaryKey(value: string) { return trimText(value).normalize('NFC').toLowerCase().normalize('NFC'); }
export function validateDictionaryInput(input: DictionaryInput): DictionaryInput {
  const forbidden = (raw: string) => /[\p{Cc}\u2028\u2029]/u.test(raw);
  const field = (raw: string, max: number) => { const value=trimText(raw); if(!value || [...value].length>max || forbidden(raw)) throw new DictionaryError('invalid'); return value; };
  const original=field(input.original,128);
  if(input.kind==='term') {
    if(input.replacement!==null) throw new DictionaryError('invalid');
    if(input.category!==null && forbidden(input.category)) throw new DictionaryError('invalid');
    return {...input,original,category: input.category && trimText(input.category) ? field(input.category,64) : null};
  }
  if(input.kind!=='correction' || input.category!==null || input.replacement===null) throw new DictionaryError('invalid');
  return {...input,original,replacement:field(input.replacement,128)};
}
export function dictionaryError(error: unknown): DictionaryError {
  if(error instanceof DictionaryError) return error;
  const codes: DictionaryErrorCode[]=['invalid','duplicate','notFound','conflict','busy','unavailable','forbidden'];
  if(typeof error==='object' && error!==null && 'code' in error && codes.includes(error.code as DictionaryErrorCode)) return new DictionaryError(error.code as DictionaryErrorCode,'existingId' in error && typeof error.existingId==='string' ? error.existingId : undefined);
  return new DictionaryError('unavailable');
}
function object(value: unknown): Record<string,unknown> { if(typeof value!=='object'||value===null) throw new DictionaryError('unavailable'); return value as Record<string,unknown>; }
function integer(value: unknown, min=0): value is number { return typeof value==='number'&&Number.isSafeInteger(value)&&value>=min; }
export function parseDictionaryRecord(value: unknown): DictionaryRecord {
  const v=object(value);
  if((v.kind!=='term'&&v.kind!=='correction')||typeof v.id!=='string'||typeof v.original!=='string'||!(v.category===null||typeof v.category==='string')||!(v.replacement===null||typeof v.replacement==='string')||!integer(v.revision,1)||!integer(v.createdAt)||!integer(v.updatedAt)) throw new DictionaryError('unavailable');
  return {id:v.id,kind:v.kind,original:v.original,category:v.category,replacement:v.replacement,revision:v.revision,createdAt:v.createdAt,updatedAt:v.updatedAt};
}
export function parseDictionaryPage(value: unknown): DictionaryPage {
  const v=object(value); if(!Array.isArray(v.records)||v.records.length>50||!integer(v.total)||!integer(v.page,1)) throw new DictionaryError('unavailable');
  return {records:v.records.map(parseDictionaryRecord),total:v.total,page:v.page};
}
export function parseDictionaryStatus(value: unknown): DictionaryStatus {
  const v=object(value); if(!['preparing','ready','unavailable'].includes(String(v.state))||!integer(v.schemaVersion)||!integer(v.normalizationVersion)) throw new DictionaryError('unavailable');
  return {state:v.state as DictionaryStatus['state'],error:v.error===null?null:{code:dictionaryError(v.error).code},schemaVersion:v.schemaVersion,normalizationVersion:v.normalizationVersion};
}
