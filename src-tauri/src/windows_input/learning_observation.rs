//! Optional read-only correction evidence. No input permission or clipboard APIs.
use super::{Anchor, composition::ObservationSink};
use crate::contracts::language::{EditEvidence, LearningEligibility};
use std::{
    collections::{HashSet, VecDeque},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
mod field_reader;

const LIMIT: usize = 65536;
const MAX_SCOPES: usize = 16;
const MAX_AGE: Duration = Duration::from_secs(60);
const QUIET: Duration = Duration::from_secs(15);
type RegionKey = (u64, String, u64, u64);
type LocatedSeed = (usize, usize, usize, OutputSeed);

#[derive(Clone)]
pub(super) struct OutputSeed {
    pub task: String,
    pub turn: u64,
    pub operation: u64,
    pub revision: u64,
    pub region: String,
    pub raw: String,
    pub text: String,
}
impl OutputSeed {
    fn same_output(&self, other: &Self) -> bool {
        self.task == other.task
            && self.turn == other.turn
            && self.revision == other.revision
            && self.region == other.region
            && self.raw == other.raw
            && self.text == other.text
    }
    fn key(&self) -> RegionKey {
        (
            self.turn,
            self.region.clone(),
            self.operation,
            self.revision,
        )
    }
    fn evidence(
        &self,
        after: String,
        sequence: u64,
        sink: &ObservationSink,
        changed: Instant,
    ) -> EditEvidence {
        EditEvidence {
            id: String::new(),
            task: self.task.clone(),
            turn: self.turn,
            operation: self.operation,
            sequence,
            region: self.region.clone(),
            expires_at: crate::data::repository::now() + 600_000
                - changed.elapsed().as_millis().min(600_000) as i64,
            application: sink.snapshot.application.clone(),
            eligibility: LearningEligibility::capture(&sink.snapshot.settings),
            raw: self.raw.clone(),
            before: self.text.clone(),
            after,
        }
    }
}

struct Sources {
    version: u64,
    writing: bool,
    seeds: VecDeque<OutputSeed>,
    at: Instant,
    field_regions: HashSet<RegionKey>,
    field_version: u64,
    pending_native: VecDeque<(EditEvidence, u64, Instant)>,
    resolved: VecDeque<([u8; 32], EvidenceOrigin)>,
    cached: Vec<FieldCandidate>,
    cached_activity: u64,
    cached_input: crate::input_state::ObservationInput,
    closed_regions: HashSet<RegionKey>,
}
#[derive(Clone, Copy, PartialEq)]
enum EvidenceOrigin {
    Native,
    Field,
}
impl Sources {
    fn resolve(&mut self, signature: [u8; 32], origin: EvidenceOrigin) -> bool {
        if self.resolved.iter().any(|(key, _)| *key == signature) {
            return false;
        }
        if self.resolved.len() >= MAX_SCOPES * 2 {
            self.resolved.pop_front();
        }
        self.resolved.push_back((signature, origin));
        true
    }
}
impl Default for Sources {
    fn default() -> Self {
        Self {
            version: 0,
            writing: false,
            seeds: VecDeque::new(),
            at: Instant::now(),
            field_regions: HashSet::new(),
            field_version: 0,
            pending_native: VecDeque::new(),
            resolved: VecDeque::new(),
            cached: Vec::new(),
            cached_activity: crate::input_state::activity(),
            cached_input: crate::input_state::observation_input_stamp(),
            closed_regions: HashSet::new(),
        }
    }
}
struct Session {
    sink: Arc<ObservationSink>,
    anchor: Anchor,
    state: AtomicU8, // 0 observe, 1 settle already captured edit, 2 discard
    finished: AtomicBool,
    sources: Mutex<Sources>,
    reversal: u64,
}
pub(super) struct FieldObserver(Arc<Session>);
impl FieldObserver {
    pub fn start(sink: Arc<ObservationSink>, anchor: Anchor) -> Option<Self> {
        static READER: OnceLock<Option<mpsc::SyncSender<Arc<Session>>>> = OnceLock::new();
        let sender = READER
            .get_or_init(|| {
                let (sender, receiver) = mpsc::sync_channel(1);
                std::thread::Builder::new()
                    .name("sayweave-learning-readonly".into())
                    .spawn(move || reader(receiver))
                    .ok()
                    .map(|_| sender)
            })
            .as_ref()?;
        let session = Arc::new(Session {
            sink,
            anchor,
            state: AtomicU8::new(0),
            finished: AtomicBool::new(false),
            sources: Mutex::new(Sources::default()),
            reversal: crate::input_state::reversal_epoch(),
        });
        sender.try_send(session.clone()).ok()?;
        Some(Self(session))
    }
    pub fn begin_write(&self) {
        if let Ok(mut sources) = self.0.sources.lock() {
            sources.version += 1;
            sources.writing = true;
        }
    }
    pub fn delivered(&self, seed: OutputSeed) {
        if seed.text.is_empty()
            || seed.raw.is_empty()
            || seed.text.len() >= LIMIT
            || seed.raw.len() >= LIMIT
        {
            return;
        }
        if let Ok(mut sources) = self.0.sources.lock() {
            let writing = sources.writing;
            #[cfg(test)]
            eprintln!(
                "owned_learning_seed turn={} operation={} revision={} region={} chars={}",
                seed.turn,
                seed.operation,
                seed.revision,
                seed.region,
                seed.text.chars().count()
            );
            let same = |s: &OutputSeed| {
                s.turn == seed.turn
                    && s.region == seed.region
                    && (seed.region == "tail" || s.operation == seed.operation)
            };
            if let Some(old) = sources.seeds.iter().find(|s| same(s))
                && old.operation == seed.operation
                && old.text == seed.text
                && old.raw == seed.raw
                && old.revision == seed.revision
                && !writing
            {
                return;
            }
            sources.seeds.retain(|old| {
                !same(old)
                    && !(seed.region == "prefix" && old.turn == seed.turn && old.region == "tail")
            });
            if sources.seeds.len() >= MAX_SCOPES {
                drop(sources);
                self.stop(true);
                return;
            }
            sources.seeds.push_back(seed);
            sources.version += 1;
            sources.writing = false;
            sources.at = Instant::now();
        }
    }
    pub fn live(&self) -> bool {
        self.0.state.load(Ordering::Acquire) == 0
            && !self.0.finished.load(Ordering::Acquire)
            && self.0.reversal == crate::input_state::reversal_epoch()
            && self
                .0
                .sources
                .lock()
                .is_ok_and(|s| s.at.elapsed() < MAX_AGE)
    }
    pub fn stop(&self, settle: bool) {
        // Never wait for a UIA call. A discarded task cannot later be settled.
        if settle {
            let _ = self
                .0
                .state
                .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
        } else {
            self.0.state.store(2, Ordering::Release);
        }
        if settle {
            self.0.emit_shared_cache();
        }
        self.0.release_native(true);
    }
    pub fn native_evidence(&self, evidence: EditEvidence, revision: u64) {
        self.0.native_evidence(evidence, revision);
    }
    pub fn poll_native(&self) {
        self.0.release_native(false);
    }
}
impl Drop for FieldObserver {
    fn drop(&mut self) {
        let _ = self
            .0
            .state
            .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire);
    }
}

fn native_signature(evidence: &EditEvidence, revision: u64) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(
        serde_json::to_vec(&(
            &evidence.task,
            evidence.turn,
            &evidence.region,
            // A handoff receipt can change operation without writing again.
            // Exact receipt validation happens before this deduplication key;
            // the native revision still separates actual writes.
            revision,
            &evidence.raw,
            &evidence.before,
            &evidence.after,
            &evidence.eligibility,
        ))
        .expect("plain evidence signature serializes"),
    )
    .into()
}
impl Session {
    fn refresh_receipts(
        &self,
        scopes: &mut [Scope],
        previous: u64,
        version: u64,
        sequence: u64,
    ) -> bool {
        let Ok(mut sources) = self.sources.lock() else {
            return false;
        };
        if sources.version != version
            || !refresh_receipts(scopes, &sources.seeds, previous, version, sources.writing)
        {
            return false;
        }
        // Rebind already captured candidates under the same source fence.
        // Preserve their physical-input stamp and original quiet deadline so
        // immediate task closure cannot lose them between UIA polls.
        sources.cached = scopes
            .iter()
            .filter_map(|scope| scope.candidate(sequence + 1, &self.sink))
            .filter(|next| {
                sources.cached.iter().any(|old| {
                    native_signature(&old.evidence, old.revision)
                        == native_signature(&next.evidence, next.revision)
                })
            })
            .collect();
        sources.field_regions = scopes
            .iter()
            .flat_map(|s| s.receipts().map(OutputSeed::key))
            .collect();
        sources.field_version = version;
        true
    }
    fn evidence_allowed(&self) -> bool {
        self.state.load(Ordering::Acquire) != 2
            && self.sink.allowed()
            && self.reversal == crate::input_state::reversal_epoch()
    }
    fn native_evidence(&self, evidence: EditEvidence, revision: u64) {
        if !self.evidence_allowed() {
            return;
        }
        if let Ok(mut sources) = self.sources.lock() {
            #[cfg(test)]
            eprintln!(
                "owned_learning_native turn={} operation={} revision={} region={} covered={} field_version={} version={} writing={}",
                evidence.turn,
                evidence.operation,
                revision,
                evidence.region,
                sources.field_regions.contains(&(
                    evidence.turn,
                    evidence.region.clone(),
                    evidence.operation,
                    revision
                )),
                sources.field_version,
                sources.version,
                sources.writing
            );
            #[cfg(test)]
            eprintln!("owned_learning_regions keys={:?}", sources.field_regions);
            if sources.closed_regions.contains(&(
                evidence.turn,
                evidence.region.clone(),
                evidence.operation,
                revision,
            )) {
                return;
            }
            let signature = native_signature(&evidence, revision);
            if sources.resolved.iter().any(|(key, _)| *key == signature) {
                return;
            }
            if sources
                .pending_native
                .iter()
                .any(|(e, revision, _)| native_signature(e, *revision) == signature)
            {
                return;
            }
            // Attaching a field baseline does not prove a correction. Keep
            // native facts until the field actually settles or retires.
            if !self.finished.load(Ordering::Acquire)
                && !sources.writing
                && sources.field_version == sources.version
                && sources.field_regions.contains(&(
                    evidence.turn,
                    evidence.region.clone(),
                    evidence.operation,
                    revision,
                ))
                && sources.pending_native.len() < MAX_SCOPES
            {
                sources
                    .pending_native
                    .push_back((evidence, revision, Instant::now()));
                return;
            }
            if !sources.resolve(signature, EvidenceOrigin::Native) {
                return;
            }
        }
        if self.evidence_allowed() {
            self.sink.emit_evidence(evidence);
        }
    }
    fn emit_field(&self, candidate: FieldCandidate) {
        let ready = self
            .sources
            .lock()
            .is_ok_and(|mut sources| candidate.claim(&mut sources));
        if ready && self.evidence_allowed() {
            self.sink.emit_evidence(candidate.evidence);
        }
    }
    fn emit_shared_cache(&self) {
        // The latest verified candidates contain only attributed own text.
        // A new task can settle them locally, without waiting for UIA.
        let ready = self.sources.lock().map(|mut sources| {
            let cached = std::mem::take(&mut sources.cached);
            if sources.cached_activity != crate::input_state::activity() {
                return Vec::new();
            }
            cached
                .into_iter()
                .filter(|c| c.claim(&mut sources))
                .collect::<Vec<_>>()
        });
        if let Ok(ready) = ready {
            for candidate in ready {
                if self.evidence_allowed() {
                    self.sink.emit_evidence(candidate.evidence);
                }
            }
        }
    }
    fn settle_field_boundary(&self) -> bool {
        if !self.evidence_allowed() {
            return false;
        }
        let ready = self.sources.lock().map(|mut sources| {
            #[cfg(test)]
            eprintln!("owned_learning_boundary writing={} field_version={} version={} cached={} input={:?} now={:?}",sources.writing,sources.field_version,sources.version,sources.cached.len(),sources.cached_input,crate::input_state::observation_input_stamp());
            if sources.writing
                || sources.field_version != sources.version
                || !crate::input_state::observation_input_stamp()
                    .boundary_after(sources.cached_input)
            {
                return None;
            }
            let cached = std::mem::take(&mut sources.cached);
            let ready = cached
                .into_iter()
                .filter(|candidate| candidate.claim(&mut sources))
                .collect::<Vec<_>>();
            // A verified delivery boundary retires these regions even when
            // the correction already settled or the field was unchanged.
            // Their native ranges may now collapse into host placeholders.
            sources.closed_regions = sources.field_regions.clone();
            let closed = sources.closed_regions.clone();
            sources.pending_native.retain(|(e, revision, _)| {
                !closed.contains(&(e.turn, e.region.clone(), e.operation, *revision))
            });
            Some(ready)
        });
        let Ok(Some(ready)) = ready else {
            return false;
        };
        for candidate in ready {
            if self.evidence_allowed() {
                self.sink.emit_evidence(candidate.evidence);
            }
        }
        true
    }
    fn release_native(&self, all: bool) {
        let pending = self.sources.lock().map(|mut s| {
            if all {
                s.field_regions.clear();
            }
            let mut ready = Vec::new();
            let mut waiting = VecDeque::new();
            let pending = std::mem::take(&mut s.pending_native);
            for (e, revision, at) in pending {
                if s.closed_regions
                    .contains(&(e.turn, e.region.clone(), e.operation, revision))
                {
                    continue;
                }
                if all || at.elapsed() >= QUIET + Duration::from_secs(2) {
                    if s.resolve(native_signature(&e, revision), EvidenceOrigin::Native) {
                        ready.push(e);
                    }
                } else {
                    waiting.push_back((e, revision, at));
                }
            }
            s.pending_native = waiting;
            ready
        });
        if self.evidence_allowed()
            && let Ok(pending) = pending
        {
            for evidence in pending {
                if self.evidence_allowed() {
                    self.sink.emit_evidence(evidence);
                }
            }
        }
    }
    fn finish(&self) {
        self.finished.store(true, Ordering::Release);
        if let Ok(mut sources) = self.sources.lock() {
            sources.cached.clear();
        }
        self.release_native(true);
    }
}

/// The field prefix/suffix remain private local memory. Only the own-output
/// replacement can leave this object, and repeated matches never select a scope.
struct Attribution {
    prefix: String,
    suffix: String,
    before: String,
    last: String,
    verified: bool,
    changed: Option<(String, Instant)>,
}
impl Attribution {
    fn new(field: &str, text: &str) -> Option<Self> {
        if field.len() >= LIMIT || text.is_empty() {
            return None;
        }
        let at = field.find(text)?;
        if field.rfind(text) != Some(at) {
            return None;
        }
        Some(Self {
            prefix: field[..at].into(),
            suffix: field[at + text.len()..].into(),
            before: text.into(),
            last: text.into(),
            verified: true,
            changed: None,
        })
    }
    fn read(&mut self, field: &str, now: Instant) -> Result<(), ()> {
        if field.len() >= LIMIT
            || !field.starts_with(&self.prefix)
            || !field.ends_with(&self.suffix)
            || field.len() < self.prefix.len() + self.suffix.len()
        {
            return Err(());
        }
        let after = &field[self.prefix.len()..field.len() - self.suffix.len()];
        // Snapshot settlement uses the observed text and its stability. UIA's
        // active-composition range may remain cached after native commitment.
        if after == self.before || after.is_empty() {
            self.changed = None;
        } else if self.last != after || !self.verified {
            self.changed = Some((after.into(), now));
        }
        self.last = after.into();
        self.verified = true;
        Ok(())
    }
    fn suspend(&mut self) {
        self.verified = false;
    }
    fn readable(&self) -> bool {
        self.verified
    }
    fn settled(&mut self, now: Instant, force: bool) -> Option<String> {
        if self.readable()
            && self
                .changed
                .as_ref()
                .is_some_and(|(_, at)| force || now.duration_since(*at) >= QUIET)
        {
            self.changed.take().map(|(text, _)| text)
        } else {
            None
        }
    }
}
struct Scope {
    seed: OutputSeed,
    attribution: Attribution,
    members: Vec<OutputSeed>,
}
struct FieldCandidate {
    evidence: EditEvidence,
    revision: u64,
    members: Vec<[u8; 32]>,
}
impl FieldCandidate {
    fn claim(&self, sources: &mut Sources) -> bool {
        let key = native_signature(&self.evidence, self.revision);
        sources
            .cached
            .retain(|c| native_signature(&c.evidence, c.revision) != key);
        if sources.resolved.iter().any(|(old, _)| *old == key)
            || self.members.iter().any(|signature| {
                sources
                    .resolved
                    .contains(&(*signature, EvidenceOrigin::Native))
            })
        {
            return false;
        }
        sources.resolve(key, EvidenceOrigin::Field);
        for signature in &self.members {
            sources
                .pending_native
                .retain(|(e, revision, _)| native_signature(e, *revision) != *signature);
            sources.resolve(*signature, EvidenceOrigin::Field);
        }
        true
    }
}
impl Scope {
    fn receipts(&self) -> impl Iterator<Item = &OutputSeed> {
        self.members
            .iter()
            .chain(std::iter::once(&self.seed).filter(|_| self.members.is_empty()))
    }
    fn candidate(&self, sequence: u64, sink: &ObservationSink) -> Option<FieldCandidate> {
        if !self.attribution.readable() {
            return None;
        }
        let (after, changed) = self.attribution.changed.as_ref()?;
        let mut offset = 0;
        let mut members = Vec::new();
        // Only a unique projection with the other owned members unchanged
        // can identify a native partial record as this same full correction.
        for member in &self.members {
            let prefix = &self.seed.text[..offset];
            let suffix = &self.seed.text[offset + member.text.len()..];
            if after.starts_with(prefix)
                && after.ends_with(suffix)
                && let Some(part) =
                    after.get(prefix.len()..after.len().saturating_sub(suffix.len()))
                && part != member.text
            {
                let native = member.evidence(part.into(), sequence, sink, *changed);
                members.push(native_signature(&native, member.revision));
            }
            offset += member.text.len();
        }
        if members.len() != 1 {
            members.clear();
        }
        Some(FieldCandidate {
            evidence: self.seed.evidence(after.clone(), sequence, sink, *changed),
            revision: self.seed.revision,
            members,
        })
    }
}
/// Native block boundaries are not editing boundaries. Join only unique,
/// nonoverlapping receipt spans that touch in the verified original field.
/// No field text between separate owned spans is included in the evidence.
fn attach_scopes(field: &str, seeds: VecDeque<OutputSeed>) -> (Vec<Scope>, HashSet<RegionKey>) {
    use sha2::{Digest, Sha256};
    let mut attached = seeds
        .into_iter()
        .enumerate()
        .filter_map(|(index, seed)| {
            let attribution = Attribution::new(field, &seed.text)?;
            let start = attribution.prefix.len();
            Some((start, start + seed.text.len(), index, seed))
        })
        .collect::<Vec<_>>();
    attached.sort_by_key(|item| item.0);
    let overlaps = (0..attached.len())
        .map(|index| {
            attached.iter().enumerate().any(|(other, value)| {
                other != index && attached[index].0 < value.1 && value.0 < attached[index].1
            })
        })
        .collect::<Vec<_>>();
    let mut groups: Vec<Vec<LocatedSeed>> = Vec::new();
    for (item, overlapping) in attached.into_iter().zip(overlaps) {
        if overlapping {
            continue;
        }
        if let Some(group) = groups.last_mut()
            && group
                .last()
                .is_some_and(|last| last.1 == item.0 && last.3.task == item.3.task)
        {
            group.push(item);
        } else {
            groups.push(vec![item]);
        }
    }
    let mut scopes = Vec::new();
    let mut covered = HashSet::new();
    for group in groups {
        let mut seed = group.iter().max_by_key(|item| item.2).unwrap().3.clone();
        if group.len() > 1 {
            let keys = group.iter().map(|item| item.3.key()).collect::<Vec<_>>();
            seed.region = format!(
                "field:{:x}",
                Sha256::digest(serde_json::to_vec(&keys).unwrap())
            );
            seed.text = field[group[0].0..group.last().unwrap().1].into();
            // raw remains an actual contributing receipt's ASR excerpt. Never
            // concatenate cumulative ASR sources or field/private context.
            if let Some(attribution) = Attribution::new(field, &seed.text) {
                let members = keys.into_iter().collect::<HashSet<_>>();
                covered.extend(members.iter().cloned());
                scopes.push(Scope {
                    seed,
                    attribution,
                    members: group.into_iter().map(|item| item.3).collect(),
                });
            }
        } else if let Some(attribution) = Attribution::new(field, &seed.text) {
            covered.insert(seed.key());
            scopes.push(Scope {
                seed,
                attribution,
                members: Vec::new(),
            });
        }
    }
    (scopes, covered)
}
/// Reissuing a receipt for the same delivered text is not a new text baseline.
/// One source-version step with no pending write proves this was a delivery
/// notification, rather than begin_write followed by a text mutation.
fn refresh_receipts(
    scopes: &mut [Scope],
    seeds: &VecDeque<OutputSeed>,
    previous: u64,
    current: u64,
    writing: bool,
) -> bool {
    if writing
        || previous.checked_add(1) != Some(current)
        || scopes.is_empty()
        || scopes.iter().map(|s| s.receipts().count()).sum::<usize>() != seeds.len()
    {
        return false;
    }
    let mut used = HashSet::new();
    let mut replacements = Vec::new();
    for scope in scopes.iter() {
        let mut members = Vec::new();
        for old in scope.receipts() {
            let matches = seeds
                .iter()
                .enumerate()
                .filter(|(_, seed)| old.same_output(seed))
                .collect::<Vec<_>>();
            if matches.len() != 1 || !used.insert(matches[0].0) {
                return false;
            }
            members.push(matches[0].1.clone());
        }
        replacements.push(members);
    }
    for (scope, members) in scopes.iter_mut().zip(replacements) {
        if scope.members.is_empty() {
            scope.seed = members.into_iter().next().unwrap();
        } else {
            // The synthetic field scope and its attributed edit stay the same;
            // only its concrete receipt members receive updated identities.
            scope.members = members;
        }
    }
    true
}
struct Current {
    session: Arc<Session>,
    field: field_reader::Field,
    sources_version: u64,
    scopes: Vec<Scope>,
    poll_at: Instant,
    activity: u64,
    activity_at: Instant,
    sequence: u64,
}
impl Current {
    fn emit(&mut self, force: bool) {
        emit_cached(&self.session, &mut self.scopes, &mut self.sequence, force);
    }
    fn poll(&mut self, reader: &field_reader::Reader) -> Result<(), &'static str> {
        if !self.session.sink.allowed() {
            return Err("settingsChanged");
        }
        if self.session.reversal != crate::input_state::reversal_epoch() {
            return Err("reversal");
        }
        let (version, writing, seeds, source_at) = {
            let sources = self.session.sources.lock().map_err(|_| "unavailable")?;
            (
                sources.version,
                sources.writing,
                sources.seeds.clone(),
                sources.at,
            )
        };
        let refreshed = self.session.refresh_receipts(
            &mut self.scopes,
            self.sources_version,
            version,
            self.sequence,
        );
        if refreshed {
            self.sources_version = version;
        }
        // These candidates already passed the source fence when read. A new
        // own write closes their observation boundary; it cannot turn that
        // captured user correction into a model-generated edit.
        if version != self.sources_version {
            self.emit(true);
            self.session.release_native(true);
        }
        if writing || seeds.is_empty() {
            return Ok(());
        }
        let activity = crate::input_state::activity();
        if activity != self.activity {
            self.activity = activity;
            self.activity_at = Instant::now();
        }
        let keyboard_due = self.activity_at.elapsed() >= Duration::from_millis(100)
            && self.poll_at < self.activity_at;
        let interval = if refreshed || version != self.sources_version {
            Duration::from_millis(100)
        } else {
            Duration::from_secs(1)
        };
        if !keyboard_due && self.poll_at.elapsed() < interval {
            return Ok(());
        }
        self.poll_at = Instant::now();
        let input = crate::input_state::observation_input_stamp();
        let snapshot = match reader.read(&self.session.anchor, &self.field) {
            Err("readRacedEdit") => {
                #[cfg(test)]
                eprintln!(
                    "owned_learning_snapshot result=raced scopes={}",
                    self.scopes.len()
                );
                for scope in &mut self.scopes {
                    scope.attribution.suspend();
                }
                if let Ok(mut sources) = self.session.sources.lock() {
                    sources.cached.clear();
                }
                return Ok(());
            }
            result => result?,
        };
        if activity != crate::input_state::activity()
            || input != crate::input_state::observation_input_stamp()
        {
            for scope in &mut self.scopes {
                scope.attribution.suspend();
            }
            if let Ok(mut sources) = self.session.sources.lock() {
                sources.cached.clear();
            }
            return Ok(());
        }
        #[cfg(test)]
        eprintln!(
            "owned_learning_snapshot result=read version={version} baseline={} composition_range={:?} chars={} scopes={}",
            self.sources_version,
            snapshot.composing,
            snapshot.text.chars().count(),
            self.scopes.len()
        );
        #[cfg(test)]
        if snapshot.text.chars().count() <= 2 {
            eprintln!(
                "owned_learning_short_field units={:?}",
                snapshot.text.chars().map(u32::from).collect::<Vec<_>>()
            );
        }
        let sources = self.session.sources.lock().map_err(|_| "unavailable")?;
        if sources.writing
            || sources.version != version
            || self.session.state.load(Ordering::Acquire) != 0
        {
            return Ok(());
        }
        drop(sources);
        // Only an observed field boundary plus a recent explicit physical
        // boundary can settle the prior captured edit before the quiet timer.
        // Empty after delete/retype still keeps the original baseline.
        let field_cleared = snapshot.text.chars().all(char::is_whitespace)
            || self.scopes.iter().any(|scope| {
                snapshot.text == format!("{}{}", scope.attribution.prefix, scope.attribution.suffix)
            });
        if version == self.sources_version && field_cleared && self.session.settle_field_boundary()
        {
            return Err("submitted");
        }
        // Empty is an edit snapshot, not a field-lifetime event. The original
        // scope can survive delete-then-retype within MAX_AGE. Prefix/suffix
        // checks below still discard scopes if outside content was removed.
        if version != self.sources_version {
            let latest = seeds.back().unwrap();
            let newest_attached = Attribution::new(&snapshot.text, &latest.text).is_some();
            if !newest_attached {
                if source_at.elapsed() >= Duration::from_secs(1) {
                    return Err("baselineUnavailable");
                }
                return Ok(());
            }
            let (scopes, covered) = attach_scopes(&snapshot.text, seeds);
            self.scopes = scopes;
            // Do not turn a pre-render snapshot into an attached baseline.
            if self.scopes.is_empty() {
                return Ok(());
            }
            if let Ok(mut sources) = self.session.sources.lock()
                && sources.version == version
                && !sources.writing
            {
                sources.field_regions = covered;
                sources.field_version = version;
            }
            self.sources_version = version;
            log::info!(target:"sayweave","operation=learning_field task_id={} result=baseline scopes={} revision={}",self.scopes[0].seed.task,self.scopes.len(),version);
        } else {
            let prior_changes = self
                .scopes
                .iter()
                .filter(|s| s.attribution.readable() && s.attribution.changed.is_some())
                .count();
            let previous_scopes = self.scopes.len();
            self.scopes.retain_mut(|scope| {
                let valid = scope
                    .attribution
                    .read(&snapshot.text, Instant::now())
                    .is_ok();
                #[cfg(test)]
                if !valid { eprintln!("owned_learning_snapshot result=scopeLost task={} turn={} operation={} prefix_same={} suffix_same={}",scope.seed.task,scope.seed.turn,scope.seed.operation,snapshot.text.starts_with(&scope.attribution.prefix),snapshot.text.ends_with(&scope.attribution.suffix)); }
                valid
            });
            if self.scopes.len() != previous_scopes {
                self.session.release_native(true);
            }
            if let Ok(mut sources) = self.session.sources.lock()
                && sources.version == version
                && !sources.writing
            {
                sources.field_regions = self
                    .scopes
                    .iter()
                    .flat_map(|scope| scope.receipts().map(OutputSeed::key))
                    .collect();
                sources.field_version = version;
            }
            let changes = self
                .scopes
                .iter()
                .filter(|s| s.attribution.readable() && s.attribution.changed.is_some())
                .count();
            if changes > prior_changes {
                log::info!(target:"sayweave","operation=learning_field result=correctionCaptured scopes={changes}");
            }
        }
        if let Ok(mut sources) = self.session.sources.lock()
            && sources.version == version
            && !sources.writing
            && self.session.state.load(Ordering::Acquire) == 0
        {
            sources.cached = self
                .scopes
                .iter()
                .filter_map(|scope| scope.candidate(self.sequence + 1, &self.session.sink))
                .collect();
            sources.cached_activity = activity;
            sources.cached_input = input;
        }
        self.emit(false);
        Ok(())
    }
}
fn emit_cached(session: &Session, scopes: &mut [Scope], sequence: &mut u64, force: bool) {
    if session.state.load(Ordering::Acquire) == 2
        || !session.sink.allowed()
        || session.reversal != crate::input_state::reversal_epoch()
    {
        return;
    }
    if !session
        .sources
        .lock()
        .is_ok_and(|sources| sources.cached_activity == crate::input_state::activity())
    {
        return;
    }
    for scope in scopes {
        let candidate = scope.candidate(*sequence + 1, &session.sink);
        if scope.attribution.settled(Instant::now(), force).is_some()
            && let Some(candidate) = candidate
        {
            *sequence += 1;
            session.emit_field(candidate);
        }
    }
}
fn reader(receiver: mpsc::Receiver<Arc<Session>>) {
    let Some(reader) = field_reader::Reader::new() else {
        return;
    };
    let mut current: Option<Current> = None;
    loop {
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(session) => {
                if let Some(mut old) = current.take() {
                    if old.session.state.load(Ordering::Acquire) != 2 {
                        old.emit(true);
                    }
                    old.session.finish();
                }
                if session.state.load(Ordering::Acquire) != 0 {
                    session.finish();
                    continue;
                }
                match reader.capture(&session.anchor) {
                    Ok(field) => {
                        current = Some(Current {
                            session,
                            field,
                            sources_version: 0,
                            scopes: Vec::new(),
                            poll_at: Instant::now() - Duration::from_secs(1),
                            activity: crate::input_state::activity(),
                            activity_at: Instant::now(),
                            sequence: 0,
                        })
                    }
                    Err(reason) => {
                        log::info!(target:"sayweave","operation=learning_field result=unavailable reason={reason}");
                        session.finish();
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if let Some(active) = &mut current {
            let state = active.session.state.load(Ordering::Acquire);
            let ended = state != 0
                || active
                    .session
                    .sources
                    .lock()
                    .map_or(true, |s| s.at.elapsed() >= MAX_AGE);
            let result = if ended {
                Err("ended")
            } else {
                active.poll(&reader)
            };
            if let Err(reason) = result {
                if state != 2 && matches!(reason, "targetChanged" | "fieldChanged") {
                    active.session.settle_field_boundary();
                }
                if state != 2 && matches!(reason, "ended" | "targetChanged" | "fieldChanged") {
                    active.emit(true);
                }
                log::info!(target:"sayweave","operation=learning_field result=ended reason={reason} scopes={}",active.scopes.len());
                active.session.finish();
                current = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // These fixtures exercise the real process-wide physical input counters.
    // Keep their stimuli isolated from other observation fixtures.
    static INPUT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    #[test]
    #[ignore = "explicit bounded readonly field diagnostics; no input or activation"]
    fn inspect_actual_field_readonly() {
        use sha2::{Digest, Sha256};
        let expected: u32 = std::env::var("SAYWEAVE_LEARNING_READONLY_PID")
            .expect("explicit target PID")
            .parse()
            .unwrap();
        assert!(expected > 0);
        let reader = field_reader::Reader::new().expect("readonly UIA apartment");
        let began = Instant::now();
        let mut original: Option<(Anchor, field_reader::Field)> = None;
        let mut previous: Option<String> = None;
        let mut last_line = String::new();
        println!(
            "FIELD_JSON:{}",
            serde_json::json!({"ready":true,"readonly":true,"process":expected})
        );
        while began.elapsed() < Duration::from_secs(90) {
            let value = match super::super::native::anchor() {
                Ok(anchor) if anchor.process == expected || anchor.editor.process == expected => {
                    match reader.capture(&anchor) {
                        Ok(fresh) => {
                            let original_result = original.as_ref().map(|(a, f)| reader.read(a, f));
                            let (original_ok, original_error) = match original_result {
                                Some(Ok(_)) => (true, None),
                                Some(Err(reason)) => (false, Some(reason)),
                                None => (false, None),
                            };
                            match reader.read(&anchor, &fresh) {
                                Ok(snapshot) => {
                                    let old = previous.as_deref().unwrap_or("");
                                    let same = previous.as_deref() == Some(snapshot.text.as_str());
                                    let prefix = old
                                        .chars()
                                        .zip(snapshot.text.chars())
                                        .take_while(|(a, b)| a == b)
                                        .count();
                                    let suffix = old
                                        .chars()
                                        .rev()
                                        .zip(snapshot.text.chars().rev())
                                        .take_while(|(a, b)| a == b)
                                        .count();
                                    let value = serde_json::json!({
                                        "result":"snapshot","process":anchor.process,"editor":anchor.editor.process,
                                        "originalRead":original_ok,"originalError":original_error,
                                        "composing":snapshot.composing,"chars":snapshot.text.chars().count(),
                                        "digest":format!("{:x}",Sha256::digest(snapshot.text.as_bytes())),
                                        "same":same,"commonPrefixChars":prefix,"commonSuffixChars":suffix,
                                        "trailingCr":snapshot.text.chars().rev().take_while(|c| *c=='\r').count(),
                                        "trailingLf":snapshot.text.chars().rev().take_while(|c| *c=='\n').count(),
                                        "containsSqlit":snapshot.text.contains("SQLit"),
                                        "containsSayweave":snapshot.text.to_lowercase().contains("sayweave"),
                                        "containsSeville":snapshot.text.contains("Seville"),
                                        "containsCwave":snapshot.text.contains("C wave")
                                    });
                                    previous = Some(snapshot.text);
                                    if original.is_none() {
                                        original = Some((anchor, fresh));
                                    }
                                    value
                                }
                                Err(reason) => {
                                    serde_json::json!({"result":"readFailed","reason":reason,"originalRead":original_ok,"originalError":original_error})
                                }
                            }
                        }
                        Err(reason) => {
                            serde_json::json!({"result":"captureFailed","reason":reason})
                        }
                    }
                }
                _ => serde_json::json!({"result":"waitingOriginalProcess"}),
            };
            let line = value.to_string();
            if line != last_line {
                println!(
                    "FIELD_JSON:{}",
                    serde_json::json!({"elapsedMs":began.elapsed().as_millis(),"state":value})
                );
                last_line = line;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        println!(
            "FIELD_JSON:{}",
            serde_json::json!({"finished":true,"readonly":true})
        );
    }
    #[test]
    fn own_correction_keeps_private_field_text_local() {
        let now = Instant::now();
        let mut scope =
            Attribution::new("私有前文：项目使用SQLit。原有后文", "项目使用SQLit").unwrap();
        scope
            .read("私有前文：项目使用SQLite。原有后文", now)
            .unwrap();
        assert!(scope.settled(now, false).is_none());
        assert_eq!(
            scope.settled(now + QUIET, false).as_deref(),
            Some("项目使用SQLite")
        );
    }
    #[test]
    fn duplicate_scope_outside_change_and_incomplete_snapshot_do_not_select_output() {
        assert!(Attribution::new("SQLit SQLit", "SQLit").is_none());
        assert!(Attribution::new("哈哈哈", "哈哈").is_none());
        assert!(Attribution::new("aaa", "aa").is_none());
        assert!(Attribution::new(&"a".repeat(LIMIT), "a").is_none());
        let mut scope = Attribution::new("前SQLit后", "SQLit").unwrap();
        assert!(scope.read("改SQLite后", Instant::now()).is_err());
    }
    #[test]
    fn latest_field_change_settles_by_stability_even_when_composition_metadata_is_cached() {
        let now = Instant::now();
        let mut scope = Attribution::new("Seville", "Seville").unwrap();
        scope.read("Sayweave", now).unwrap();
        scope.read("Sayweav", now + Duration::from_secs(1)).unwrap();
        assert_eq!(
            scope.changed.as_ref().map(|v| v.0.as_str()),
            Some("Sayweav"),
            "the latest attributed snapshot replaces the previous candidate"
        );
        assert!(
            scope.settled(now + Duration::from_secs(2), false).is_none(),
            "a transient snapshot does not bypass the stability interval"
        );
        assert_eq!(scope.last, "Sayweav");
        scope.read("Sayweave", now + QUIET + QUIET).unwrap();
        assert_eq!(
            scope.settled(now + QUIET + QUIET + QUIET, false).as_deref(),
            Some("Sayweave")
        );
    }
    #[test]
    fn raced_read_preserves_candidate_but_requires_a_fresh_confirmed_snapshot() {
        let now = Instant::now();
        let mut scope = Attribution::new("Seville", "Seville").unwrap();
        scope.read("Sayweave", now).unwrap();
        scope.suspend();
        assert_eq!(scope.changed.as_ref().unwrap().0, "Sayweave");
        assert!(scope.settled(now + QUIET, true).is_none());
        scope.read("Seville", now + QUIET).unwrap();
        assert!(scope.settled(now + QUIET + QUIET, true).is_none());
    }
    #[test]
    fn whole_field_delete_retype_and_ime_cancel_keep_the_receipt_baseline() {
        let now = Instant::now();
        let mut scope = Attribution::new("Seville", "Seville").unwrap();
        for text in ["Sevill", "Sevil", "Sevi", "Sev", "Se", "S", ""] {
            scope.read(text, now).unwrap();
        }
        assert!(scope.settled(now + QUIET, false).is_none());
        for text in ["s", "sa", "sayweave"] {
            scope.read(text, now + QUIET).unwrap();
            assert_eq!(scope.last, text);
            assert!(
                scope
                    .settled(now + QUIET + Duration::from_secs(1), false)
                    .is_none()
            );
        }
        scope.read("", now + QUIET + QUIET).unwrap();
        assert!(scope.settled(now + QUIET + QUIET, true).is_none());
        scope.read("Sayweave", now + QUIET + QUIET).unwrap();
        assert_eq!(scope.before, "Seville");
        assert_eq!(
            scope.settled(now + QUIET * 3, false).as_deref(),
            Some("Sayweave")
        );
    }
    #[test]
    fn transient_undo_deletion_and_repeated_notifications_do_not_create_correction() {
        let now = Instant::now();
        let mut scope = Attribution::new("SQLit", "SQLit").unwrap();
        scope.read("SQLite", now).unwrap();
        assert!(
            scope
                .settled(now + Duration::from_secs(14), false)
                .is_none()
        );
        scope.read("SQLite", now).unwrap();
        scope.read("SQLite", now + Duration::from_secs(14)).unwrap();
        assert_eq!(scope.settled(now + QUIET, false).as_deref(), Some("SQLite"));
        scope.read("SQLite", now + QUIET + QUIET).unwrap();
        assert!(scope.settled(now + QUIET + QUIET, true).is_none());
        scope.read("SQLite", now).unwrap();
        scope.read("SQLit", now).unwrap();
        assert!(scope.settled(now, true).is_none());
        scope.read("", now).unwrap();
        assert!(scope.settled(now, true).is_none());
    }
    #[test]
    fn unicode_and_real_newlines_are_preserved() {
        let mut scope = Attribution::new("前😀e\u{301}\nSQLit后", "😀e\u{301}\nSQLit").unwrap();
        scope
            .read("前😀e\u{301}\nSQLite后", Instant::now())
            .unwrap();
        assert_eq!(
            scope.settled(Instant::now(), true).unwrap(),
            "😀e\u{301}\nSQLite"
        );
    }
    #[test]
    fn adjacent_deliveries_keep_one_correction_when_a_word_is_deleted_and_retyped() {
        let mut first = seed(1, "tail");
        first.text = "S".into();
        first.raw = "S".into();
        let mut next = seed(2, "tail");
        next.turn = 2;
        next.text = "eville".into();
        next.raw = "Seville".into();
        let (mut scopes, covered) =
            attach_scopes("Seville", VecDeque::from([first.clone(), next.clone()]));
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].seed.text, "Seville");
        assert_eq!(scopes[0].seed.raw, "Seville");
        assert!(covered.contains(&first.key()) && covered.contains(&next.key()));
        let now = Instant::now();
        for text in ["Sevill", "Sevil", "Sevi", "Sev", "Se", "S", "", "Sayweave"] {
            scopes[0].attribution.read(text, now).unwrap();
        }
        assert_eq!(
            scopes[0].attribution.settled(now + QUIET, false).as_deref(),
            Some("Sayweave")
        );
    }
    #[test]
    fn separate_private_text_overlaps_and_transient_changes_do_not_form_combined_evidence() {
        let mut first = seed(1, "tail");
        first.text = "S".into();
        let mut next = seed(2, "tail");
        next.turn = 2;
        next.text = "eville".into();
        let (scopes, covered) =
            attach_scopes("S私有eville", VecDeque::from([first.clone(), next.clone()]));
        assert_eq!(scopes.len(), 2);
        assert_eq!(covered, HashSet::from([first.key(), next.key()]));
        assert_eq!(
            scopes
                .iter()
                .map(|scope| scope.seed.text.as_str())
                .collect::<Vec<_>>(),
            vec!["S", "eville"]
        );
        first.text = "Se".into();
        next.text = "eville".into();
        let (scopes, _) = attach_scopes("Seville", VecDeque::from([first.clone(), next.clone()]));
        assert!(scopes.is_empty());
        first.text = "S".into();
        let (mut scopes, _) = attach_scopes("Seville", VecDeque::from([first, next]));
        scopes[0]
            .attribution
            .read("Sayweave", Instant::now())
            .unwrap();
        assert!(
            scopes[0]
                .attribution
                .settled(Instant::now() + Duration::from_secs(1), false)
                .is_none()
        );
    }
    fn fixture() -> (FieldObserver, mpsc::Receiver<EditEvidence>) {
        use crate::contracts::language::{
            ApplicationContext, LanguageSettings, Settings, Snapshot,
        };
        let settings = LanguageSettings {
            automatic_terms: Settings {
                enabled: true,
                revision: 1,
            },
            personalization: Settings {
                enabled: true,
                revision: 1,
            },
        };
        let (send, receive) = mpsc::channel();
        let sink = Arc::new(ObservationSink {
            snapshot: Snapshot {
                settings: settings.clone(),
                application: ApplicationContext::from_identity(None, "owned-learning.exe"),
                profile_version: 0,
                shared_version: 0,
                preferences: vec![],
            },
            control: Arc::new(crate::language_policy::LearningControl::new(settings)),
            emit: Arc::new(move |e| {
                send.send(e).unwrap();
            }),
        });
        let anchor = Anchor {
            capture_id: 1,
            window: 1,
            focus: 1,
            process: 1,
            lifetime: 1,
            editor: super::super::EditorEndpoint {
                window: 1,
                process: 1,
                thread: 1,
                lifetime: 1,
            },
            generation: 1,
            captured_tick: 0,
            activity: 0,
        };
        (
            FieldObserver(Arc::new(Session {
                sink,
                anchor,
                state: AtomicU8::new(0),
                finished: AtomicBool::new(false),
                sources: Mutex::new(Sources::default()),
                reversal: crate::input_state::reversal_epoch(),
            })),
            receive,
        )
    }
    fn captured_word(observer: &FieldObserver) -> OutputSeed {
        let mut seed = seed(1, "tail");
        seed.text = "Seville".into();
        seed.raw = "Seville".into();
        observer.delivered(seed.clone());
        let mut attribution = Attribution::new("Seville", "Seville").unwrap();
        attribution.read("Sayweave", Instant::now()).unwrap();
        let scope = Scope {
            seed: seed.clone(),
            attribution,
            members: Vec::new(),
        };
        let candidate = scope.candidate(1, &observer.0.sink).unwrap();
        let mut sources = observer.0.sources.lock().unwrap();
        sources.cached = vec![candidate];
        sources.cached_input = crate::input_state::observation_input_stamp();
        sources.field_version = sources.version;
        sources.field_regions.insert(seed.key());
        seed
    }
    #[test]
    fn a_single_attached_receipt_defers_native_clear_facts_until_field_settlement() {
        let _input = INPUT.blocking_lock();
        let (observer, events) = fixture();
        let mut seed = seed(1, "tail");
        seed.text = "Seville".into();
        seed.raw = seed.text.clone();
        observer.delivered(seed.clone());
        let (_, covered) = attach_scopes("Seville", VecDeque::from([seed.clone()]));
        assert!(
            covered.contains(&seed.key()),
            "single-field coverage must use its real receipt too"
        );
        {
            let mut sources = observer.0.sources.lock().unwrap();
            sources.field_version = sources.version;
            sources.field_regions = covered;
        }
        observer.0.native_evidence(
            seed.evidence("S\n".into(), 2, &observer.0.sink, Instant::now()),
            seed.revision,
        );
        assert!(events.try_recv().is_err());
        assert_eq!(observer.0.sources.lock().unwrap().pending_native.len(), 1);
        observer.stop(false);
    }
    #[test]
    fn submit_and_pointer_boundary_settle_captured_word_before_quiet_and_close_native_region() {
        let _input = INPUT.blocking_lock();
        for pointer in [false, true] {
            let (observer, events) = fixture();
            let seed = captured_word(&observer);
            if pointer {
                crate::input_state::observation_pointer();
            } else {
                crate::input_state::edit_key_pressed(0x0d);
                crate::input_state::key_released(0x0d);
            }
            assert!(observer.0.settle_field_boundary());
            let evidence = events
                .try_recv()
                .expect("captured correction settles before 15-second quiet");
            assert_eq!(
                (evidence.before.as_str(), evidence.after.as_str()),
                ("Seville", "Sayweave")
            );
            observer.0.native_evidence(
                seed.evidence("S\n".into(), 2, &observer.0.sink, Instant::now()),
                seed.revision,
            );
            assert!(observer.0.settle_field_boundary());
            observer.stop(true);
            assert!(events.try_recv().is_err());
        }
    }
    #[test]
    fn unread_typing_deletion_and_source_changes_do_not_force_old_cache() {
        let _input = INPUT.blocking_lock();
        for change in 0..4 {
            let (observer, events) = fixture();
            captured_word(&observer);
            match change {
                0 => crate::input_state::edit_key_pressed(0x08),
                1 => {
                    crate::input_state::edit_key_pressed(0x41);
                    crate::input_state::edit_key_pressed(0x0d);
                }
                2 => {
                    crate::input_state::edit_key_pressed(0x0d);
                    observer.0.sources.lock().unwrap().version += 1;
                }
                _ => {
                    crate::input_state::edit_key_pressed(0x0d);
                    observer.0.sources.lock().unwrap().writing = true;
                }
            }
            assert!(!observer.0.settle_field_boundary(), "change {change}");
            assert!(events.try_recv().is_err());
            assert!(observer.0.sources.lock().unwrap().closed_regions.is_empty());
            observer.stop(false);
        }
        crate::input_state::reset_edit_keys();
    }
    #[test]
    fn a_verified_submission_closes_native_regions_even_without_a_new_correction() {
        let _input = INPUT.blocking_lock();
        for already_settled in [false, true] {
            let (observer, events) = fixture();
            let seed = captured_word(&observer);
            if already_settled {
                observer.0.emit_shared_cache();
                assert_eq!(events.try_recv().unwrap().after, "Sayweave");
            } else {
                // An unchanged/restored field has no lexical candidate.
                observer.0.sources.lock().unwrap().cached.clear();
            }
            crate::input_state::edit_key_pressed(0x0d);
            crate::input_state::key_released(0x0d);
            assert!(observer.0.settle_field_boundary());
            observer.0.native_evidence(
                seed.evidence("S\n".into(), 2, &observer.0.sink, Instant::now()),
                seed.revision,
            );
            observer.stop(true);
            observer.0.finish();
            assert!(
                events.try_recv().is_err(),
                "submitted native placeholders must stay retired; quiet={already_settled}"
            );
        }
    }
    fn combined_fixture(observer: &FieldObserver) -> (Vec<Scope>, OutputSeed) {
        let mut first = seed(1, "tail");
        first.text = "S".into();
        let mut next = seed(2, "tail");
        next.turn = 2;
        next.text = "eville".into();
        next.raw = "Seville".into();
        observer.delivered(first.clone());
        observer.delivered(next.clone());
        let (scopes, covered) = attach_scopes("Seville", VecDeque::from([first, next.clone()]));
        let mut sources = observer.0.sources.lock().unwrap();
        sources.field_regions = covered;
        sources.field_version = sources.version;
        (scopes, next)
    }
    #[test]
    fn native_facts_wait_for_actual_field_evidence_and_survive_reader_failure() {
        let _input = INPUT.blocking_lock();
        let (observer, events) = fixture();
        let (_, next) = combined_fixture(&observer);
        let evidence = next.evidence("ayweave".into(), 1, &observer.0.sink, Instant::now());
        observer.native_evidence(evidence, next.revision);
        assert!(events.try_recv().is_err());
        observer.0.finish();
        let native = events
            .try_recv()
            .expect("baseline alone cannot consume native facts");
        assert_eq!(native.before, "eville");
        assert_eq!(native.after, "ayweave");
        assert!(events.try_recv().is_err());
    }
    #[test]
    fn actual_combined_correction_deduplicates_only_exact_member_and_late_receipt() {
        let _input = INPUT.blocking_lock();
        let (observer, events) = fixture();
        let (mut scopes, next) = combined_fixture(&observer);
        let changed = Instant::now() - QUIET;
        let native = next.evidence("ayweave".into(), 1, &observer.0.sink, changed);
        observer.native_evidence(native.clone(), next.revision);
        scopes[0].attribution.read("Sayweave", changed).unwrap();
        emit_cached(&observer.0, &mut scopes, &mut 0, false);
        let combined = events.try_recv().unwrap();
        assert_eq!(
            (combined.before.as_str(), combined.after.as_str()),
            ("Seville", "Sayweave")
        );
        observer.native_evidence(native, next.revision);
        observer.0.finish();
        assert!(
            events.try_recv().is_err(),
            "same settled member must not classify twice"
        );
    }
    #[test]
    fn stale_coverage_and_nonmatching_records_are_not_suppressed() {
        let _input = INPUT.blocking_lock();
        let (observer, events) = fixture();
        let (mut scopes, next) = combined_fixture(&observer);
        let changed = Instant::now() - QUIET;
        let native = next.evidence("other".into(), 1, &observer.0.sink, changed);
        observer.native_evidence(native.clone(), next.revision + 1);
        assert_eq!(
            events.try_recv().unwrap().after,
            "other",
            "different revision bypasses stale coverage"
        );
        observer.begin_write();
        observer.native_evidence(native.clone(), next.revision);
        assert!(
            events.try_recv().is_ok(),
            "a writing baseline cannot defer facts"
        );
        {
            let mut sources = observer.0.sources.lock().unwrap();
            sources.writing = false;
            sources.field_version = sources.version;
        }
        let mut different = native;
        different.after = "another".into();
        observer.native_evidence(different, next.revision);
        scopes[0].attribution.read("Sayweave", changed).unwrap();
        emit_cached(&observer.0, &mut scopes, &mut 0, false);
        assert_eq!(events.try_recv().unwrap().after, "Sayweave");
        observer.0.finish();
        assert_eq!(
            events.try_recv().unwrap().after,
            "another",
            "a different member edit retains its fact"
        );
    }
    #[test]
    fn bounded_pending_facts_expire_without_waiting_for_uia_and_discard_still_wins() {
        let _input = INPUT.blocking_lock();
        let (observer, events) = fixture();
        let (_, next) = combined_fixture(&observer);
        for sequence in 0..=MAX_SCOPES {
            observer.native_evidence(
                next.evidence(
                    format!("correction{sequence}"),
                    sequence as u64,
                    &observer.0.sink,
                    Instant::now(),
                ),
                next.revision,
            );
        }
        assert!(
            events.try_recv().is_ok(),
            "overflow is emitted, never dropped"
        );
        {
            let mut sources = observer.0.sources.lock().unwrap();
            for (_, _, at) in &mut sources.pending_native {
                *at -= QUIET + Duration::from_secs(2);
            }
        }
        observer.poll_native();
        assert_eq!(events.try_iter().count(), MAX_SCOPES);
        observer.native_evidence(
            next.evidence("new".into(), 99, &observer.0.sink, Instant::now()),
            next.revision,
        );
        observer.stop(false);
        observer.0.finish();
        assert!(
            events.try_recv().is_err(),
            "cancelled observations cannot publish buffered facts"
        );
    }
    #[test]
    fn next_task_settles_captured_full_word_before_releasing_partial_native_facts() {
        let _input = INPUT.blocking_lock();
        let (observer, events) = fixture();
        let (mut scopes, next) = combined_fixture(&observer);
        let changed = Instant::now();
        observer.native_evidence(
            next.evidence("ayweave".into(), 1, &observer.0.sink, changed),
            next.revision,
        );
        scopes[0].attribution.read("Sayweave", changed).unwrap();
        observer.0.sources.lock().unwrap().cached =
            vec![scopes[0].candidate(1, &observer.0.sink).unwrap()];
        observer.stop(true);
        let evidence = events.try_recv().unwrap();
        assert_eq!(
            (evidence.before.as_str(), evidence.after.as_str()),
            ("Seville", "Sayweave")
        );
        emit_cached(&observer.0, &mut scopes, &mut 0, true);
        observer.0.finish();
        assert!(events.try_recv().is_err());
    }
    #[test]
    fn native_release_wins_before_its_callback_and_field_cannot_classify_it_again() {
        let _input = INPUT.blocking_lock();
        let (mut observer, events) = fixture();
        let (entered_send, entered) = mpsc::channel();
        let (resume_send, resume) = mpsc::channel();
        let resume = Mutex::new(resume);
        let sink = Arc::get_mut(&mut Arc::get_mut(&mut observer.0).unwrap().sink).unwrap();
        let emit = sink.emit.clone();
        sink.emit = Arc::new(move |e| {
            entered_send.send(()).unwrap();
            resume.lock().unwrap().recv().unwrap();
            emit(e);
        });
        let (mut scopes, next) = combined_fixture(&observer);
        let changed = Instant::now() - QUIET;
        observer.native_evidence(
            next.evidence("ayweave".into(), 1, &observer.0.sink, changed),
            next.revision,
        );
        let session = observer.0.clone();
        let releasing = std::thread::spawn(move || session.release_native(true));
        entered.recv_timeout(Duration::from_secs(1)).unwrap();
        scopes[0].attribution.read("Sayweave", changed).unwrap();
        emit_cached(&observer.0, &mut scopes, &mut 0, false);
        assert!(
            events.try_recv().is_err(),
            "field loses before the native callback completes"
        );
        resume_send.send(()).unwrap();
        releasing.join().unwrap();
        assert_eq!(events.try_recv().unwrap().before, "eville");
        assert!(events.try_recv().is_err());
    }
    fn seed(operation: u64, region: &str) -> OutputSeed {
        OutputSeed {
            task: "d68d587b-b62b-4c31-9ffd-82977ca83ed3".into(),
            turn: 1,
            operation,
            revision: operation,
            region: region.into(),
            raw: "项目使用SQLit".into(),
            text: "项目使用SQLit".into(),
        }
    }
    #[test]
    fn writer_fence_coalesces_tail_keeps_prefix_sources_and_does_not_extend_unchanged_deadline() {
        let _input = INPUT.blocking_lock();
        let (observer, _) = fixture();
        observer.delivered(seed(1, "tail"));
        let (version, at) = {
            let sources = observer.0.sources.lock().unwrap();
            (sources.version, sources.at)
        };
        observer.delivered(seed(1, "tail"));
        {
            let sources = observer.0.sources.lock().unwrap();
            assert_eq!(sources.version, version);
            assert_eq!(sources.at, at);
        }
        observer.begin_write();
        assert!(observer.0.sources.lock().unwrap().writing);
        observer.delivered(seed(2, "tail"));
        observer.delivered(seed(3, "prefix"));
        observer.delivered(seed(4, "prefix"));
        let sources = observer.0.sources.lock().unwrap();
        assert!(!sources.writing);
        assert_eq!(sources.seeds.len(), 2);
        assert_eq!(sources.seeds[0].operation, 3);
    }
    #[test]
    fn same_body_commit_retains_its_new_receipt_operation_for_native_coverage() {
        let _input = INPUT.blocking_lock();
        let (observer, _) = fixture();
        let first = seed(4, "tail");
        let mut next = first.clone();
        next.operation = 5;
        observer.delivered(first);
        observer.delivered(next);
        let sources = observer.0.sources.lock().unwrap();
        assert_eq!(sources.seeds.len(), 1);
        assert_eq!(
            sources.seeds[0].operation, 5,
            "same text cannot erase a distinct delivery identity"
        );
    }
    #[test]
    fn handoff_receipt_preserves_correction_baseline_and_deduplicates_native_result() {
        let _input = INPUT.blocking_lock();
        let (observer, events) = fixture();
        let first = seed(4, "tail");
        let mut next = first.clone();
        next.operation = 5;
        observer.delivered(first.clone());
        let (mut scopes, _) = attach_scopes(&first.text, VecDeque::from([first.clone()]));
        scopes[0]
            .attribution
            .read("项目使用SQLite", Instant::now())
            .unwrap();
        let changed = scopes[0].attribution.changed.as_ref().unwrap().1;
        observer.delivered(next.clone());
        assert!(refresh_receipts(
            &mut scopes,
            &VecDeque::from([next.clone()]),
            1,
            2,
            false
        ));
        assert_eq!(scopes[0].attribution.before, first.text);
        assert_eq!(scopes[0].attribution.changed.as_ref().unwrap().1, changed);
        assert_eq!(scopes[0].seed.operation, 5);
        let candidate = scopes[0].candidate(1, &observer.0.sink).unwrap();
        observer.0.emit_field(candidate);
        assert_eq!(events.try_recv().unwrap().after, "项目使用SQLite");
        observer.native_evidence(
            first.evidence("项目使用SQLite".into(), 1, &observer.0.sink, changed),
            first.revision,
        );
        assert!(
            events.try_recv().is_err(),
            "same write must not classify twice across receipt operations"
        );
    }
    #[test]
    fn receipt_refresh_rejects_intervening_writes_changed_sources_and_ambiguous_members() {
        let _input = INPUT.blocking_lock();
        let first = seed(4, "tail");
        let mut next = first.clone();
        next.operation = 5;
        let (mut scopes, _) = attach_scopes(&first.text, VecDeque::from([first.clone()]));
        assert!(!refresh_receipts(
            &mut scopes,
            &VecDeque::from([next.clone()]),
            1,
            3,
            false
        ));
        assert!(!refresh_receipts(
            &mut scopes,
            &VecDeque::from([next.clone()]),
            1,
            2,
            true
        ));
        next.revision += 1;
        assert!(!refresh_receipts(
            &mut scopes,
            &VecDeque::from([next.clone()]),
            1,
            2,
            false
        ));
        next.revision = first.revision;
        next.raw.push('！');
        assert!(!refresh_receipts(
            &mut scopes,
            &VecDeque::from([next.clone()]),
            1,
            2,
            false
        ));
        assert!(!refresh_receipts(
            &mut scopes,
            &VecDeque::from([first.clone(), first.clone()]),
            1,
            2,
            false
        ));
        assert_eq!(
            scopes[0].seed.operation, 4,
            "failed refresh cannot partly rebind a scope"
        );
    }
    #[test]
    fn receipt_refresh_keeps_captured_full_word_at_immediate_next_task_boundary() {
        let _input = INPUT.blocking_lock();
        let (observer, events) = fixture();
        let first = seed(4, "tail");
        observer.delivered(first.clone());
        let (mut scopes, _) = attach_scopes(&first.text, VecDeque::from([first.clone()]));
        scopes[0]
            .attribution
            .read("项目使用SQLite", Instant::now())
            .unwrap();
        let candidate = scopes[0].candidate(1, &observer.0.sink).unwrap();
        let (stamp, at) = {
            let mut sources = observer.0.sources.lock().unwrap();
            sources.cached.push(candidate);
            (
                sources.cached_input,
                scopes[0].attribution.changed.as_ref().unwrap().1,
            )
        };
        let mut next = first;
        next.operation = 5;
        observer.delivered(next);
        assert!(observer.0.refresh_receipts(&mut scopes, 1, 2, 0));
        {
            let sources = observer.0.sources.lock().unwrap();
            assert_eq!(sources.cached.len(), 1);
            assert_eq!(sources.cached[0].evidence.operation, 5);
            assert_eq!(sources.cached_input, stamp);
            assert_eq!(scopes[0].attribution.changed.as_ref().unwrap().1, at);
        }
        observer.stop(true);
        let evidence = events
            .try_recv()
            .expect("next task retains the complete cached correction");
        assert_eq!(evidence.before, "项目使用SQLit");
        assert_eq!(evidence.after, "项目使用SQLite");
        assert!(events.try_recv().is_err());
    }
    #[test]
    fn discard_cannot_be_reactivated_and_settle_drop_keeps_only_the_reader_reference() {
        let _input = INPUT.blocking_lock();
        let (observer, _) = fixture();
        let session = observer.0.clone();
        observer.stop(false);
        observer.stop(true);
        observer.delivered(seed(1, "tail"));
        assert!(!observer.live());
        assert_eq!(session.state.load(Ordering::Acquire), 2);
        let (observer, _) = fixture();
        let session = observer.0.clone();
        observer.stop(true);
        drop(observer);
        assert_eq!(session.state.load(Ordering::Acquire), 1);
    }
    #[test]
    fn confirmed_correction_settles_once_at_the_next_own_write_boundary() {
        let _input = INPUT.blocking_lock();
        let (observer, events) = fixture();
        let seed = seed(2, "tail");
        observer.delivered(seed.clone());
        let mut attribution = Attribution::new(&seed.text, &seed.text).unwrap();
        attribution.read("项目使用SQLite", Instant::now()).unwrap();
        let mut scopes = vec![Scope {
            seed,
            attribution,
            members: Vec::new(),
        }];
        observer.begin_write();
        let mut sequence = 0;
        emit_cached(&observer.0, &mut scopes, &mut sequence, true);
        let evidence = events
            .try_recv()
            .expect("confirmed pre-write correction is retained");
        assert_eq!(evidence.before, "项目使用SQLit");
        assert_eq!(evidence.after, "项目使用SQLite");
        emit_cached(&observer.0, &mut scopes, &mut sequence, true);
        assert!(events.try_recv().is_err());
        scopes[0]
            .attribution
            .read("项目使用SQLite其他", Instant::now())
            .unwrap();
        emit_cached(&observer.0, &mut scopes, &mut sequence, false);
        assert!(
            events.try_recv().is_err(),
            "a new text change restarts the stability interval"
        );
        scopes[0]
            .attribution
            .read("项目使用SQLite其他", Instant::now())
            .unwrap();
        observer.stop(false);
        emit_cached(&observer.0, &mut scopes, &mut sequence, true);
        assert!(
            events.try_recv().is_err(),
            "discarded observation cannot settle cached text"
        );
    }
    #[test]
    fn next_task_does_not_publish_a_cache_with_unprocessed_input_activity() {
        let _input = INPUT.blocking_lock();
        let (observer, events) = fixture();
        let seed = seed(2, "tail");
        let mut attribution = Attribution::new(&seed.text, &seed.text).unwrap();
        attribution.read("项目使用SQLite", Instant::now()).unwrap();
        let scope = Scope {
            seed,
            attribution,
            members: Vec::new(),
        };
        let candidate = scope.candidate(1, &observer.0.sink).unwrap();
        let mut sources = observer.0.sources.lock().unwrap();
        sources.cached.push(candidate);
        sources.cached_activity = crate::input_state::activity().wrapping_add(1);
        drop(sources);
        observer.stop(true);
        assert!(
            events.try_recv().is_err(),
            "read again before settling after another edit"
        );
    }
    #[tokio::test]
    async fn both_reader_results_share_one_store_identity_and_one_next_dictionary_snapshot() {
        let _input = INPUT.lock().await;
        let (observer, events) = fixture();
        let seed = seed(2, "tail");
        let mut scope = Attribution::new("私有前文：项目使用SQLit。私有后文", &seed.text).unwrap();
        let changed = Instant::now() - Duration::from_secs(16);
        scope
            .read("私有前文：项目使用SQLite。私有后文", changed)
            .unwrap();
        let after = scope.settled(Instant::now(), false).unwrap();
        observer
            .0
            .sink
            .emit_evidence(seed.evidence(after.clone(), 1, &observer.0.sink, changed));
        let mut second = seed.evidence(after, 44, &observer.0.sink, changed);
        second.operation = 19;
        observer.0.sink.emit_evidence(second);
        let first = events.recv().unwrap();
        let second = events.recv().unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(first.before, "项目使用SQLit");
        assert_eq!(first.after, "项目使用SQLite");
        assert!(first.expires_at <= crate::data::repository::now() + 585_000);
        let root = tempfile::tempdir().unwrap();
        let data = crate::data::DataWorker::start(root.path().to_owned()).unwrap();
        let one = crate::language_learning::classify_observed_fixture(&data, first)
            .await
            .unwrap();
        let two = crate::language_learning::classify_observed_fixture(&data, second)
            .await
            .unwrap();
        assert_eq!(one["classifierRequests"], 1);
        assert_eq!(two["classifierRequests"], 0);
        assert_eq!(one["terms"].as_array().unwrap().len(), 1);
        assert_eq!(one["nextHotwords"], serde_json::json!(["SQLite"]));
        assert_eq!(one["nextPromptContainsTerm"], true);
        data.shutdown().await;
    }
}
