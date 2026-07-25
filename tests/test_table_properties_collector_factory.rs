// Copyright 2020 Tyler Neely
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::{
    collections::HashMap,
    env,
    ffi::{CStr, CString},
    fs,
    io::{self, Read, Write},
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    process::{Child, Command, ExitStatus, Output, Stdio},
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use rust_rocksdb::{
    BlockBasedOptions, BottommostLevelCompaction, ColumnFamilyDescriptor, CompactOptions, DB, Env,
    FlushOptions, MergeOperands, Options, WriteBatch,
    event_listener::{
        CompactionJobInfo, DBCompactionReason, DBFlushReason, EventListener, FlushJobInfo,
    },
    properties::{NUM_ENTRIES_ACTIVE_MEM_TABLE, num_files_at_level},
    table_properties_collector::{
        DBEntryType, TablePropertiesCollector, TablePropertiesCollectorCallback,
    },
    table_properties_collector_factory::{
        TablePropertiesCollectorContext, TablePropertiesCollectorFactory,
    },
};

mod util;

use util::DBPath;

const KIWI_PROPERTY_KEY: &[u8] = b"LargestLogIndex/LargestSequenceNumber";
const COLLECTOR_ID_PROPERTY_KEY: &[u8] = b"test.collector-id";
const COLLECTOR_PANIC_CASE_ENV: &str = "RUST_ROCKSDB_COLLECTOR_PANIC_CASE";
const COLLECTOR_PANIC_DB_ENV: &str = "RUST_ROCKSDB_COLLECTOR_PANIC_DB";
const EXPECTED_COLLECTOR_FACTORY_SUPPORT_ENV: &str =
    "RUST_ROCKSDB_TEST_EXPECT_COLLECTOR_FACTORY_SUPPORTED";
const CHILD_STARTED_MARKER: &str = "COLLECTOR_CHILD_STARTED";
const FLUSH_SUCCEEDED_MARKER: &str = "FLUSH_SUCCEEDED";
const OVERLAP_CHILD_ENV: &str = "RUST_ROCKSDB_COLLECTOR_OVERLAP_CHILD";
const OVERLAP_CHILD_DB_ENV: &str = "RUST_ROCKSDB_COLLECTOR_OVERLAP_DB";
const OVERLAP_CHILD_STARTED_MARKER: &str = "COLLECTOR_OVERLAP_CHILD_STARTED";
const OVERLAP_CHILD_SUCCEEDED_MARKER: &str = "COLLECTOR_OVERLAP_CHILD_SUCCEEDED";
const WATCHDOG_POLL_ERROR_CHILD_ENV: &str = "RUST_ROCKSDB_WATCHDOG_POLL_ERROR_CHILD";
const WATCHDOG_POLL_ERROR_STARTED_PATH_ENV: &str = "RUST_ROCKSDB_WATCHDOG_POLL_ERROR_STARTED_PATH";
const WATCHDOG_POLL_ERROR_PARTIAL_MARKER: &str = "WATCHDOG_POLL_ERROR_CHILD";
const WATCHDOG_POLL_ERROR_STARTED_MARKER: &str = "WATCHDOG_POLL_ERROR_CHILD_STARTED";
const INJECTED_POLL_ERROR: &str = "injected child poll error";

#[derive(Clone, Debug, Eq, PartialEq)]
struct ObservedEntry {
    key: Vec<u8>,
    value: Vec<u8>,
    entry_type: DBEntryType,
    sequence: u64,
    file_size: u64,
}

#[derive(Default)]
struct CallbackObservations {
    contexts: Mutex<Vec<TablePropertiesCollectorContext>>,
    entries: Mutex<Vec<ObservedEntry>>,
}

struct ObservingCollector {
    observations: Arc<CallbackObservations>,
}

impl TablePropertiesCollector for ObservingCollector {
    fn name(&self) -> &CStr {
        c"observing-collector"
    }

    fn add(
        &mut self,
        key: &[u8],
        value: &[u8],
        entry_type: DBEntryType,
        sequence: u64,
        file_size: u64,
    ) {
        self.observations
            .entries
            .lock()
            .expect("observed entries lock poisoned")
            .push(ObservedEntry {
                key: key.to_vec(),
                value: value.to_vec(),
                entry_type,
                sequence,
                file_size,
            });
    }

    fn finish(&mut self) -> HashMap<Vec<u8>, Vec<u8>> {
        HashMap::new()
    }
}

struct ObservingFactory {
    observations: Arc<CallbackObservations>,
}

impl TablePropertiesCollectorFactory for ObservingFactory {
    type Collector = ObservingCollector;

    fn create(&self, context: TablePropertiesCollectorContext) -> Self::Collector {
        self.observations
            .contexts
            .lock()
            .expect("observed contexts lock poisoned")
            .push(context);
        ObservingCollector {
            observations: Arc::clone(&self.observations),
        }
    }

    fn name(&self) -> &CStr {
        c"observing-factory"
    }
}

#[derive(Default)]
struct LifecycleCounts {
    next_collector_id: AtomicU64,
    factories_dropped: AtomicU64,
    collectors_created: AtomicU64,
    collectors_dropped: AtomicU64,
}

struct CountingCollector {
    id: u64,
    counts: Arc<LifecycleCounts>,
}

impl TablePropertiesCollector for CountingCollector {
    fn name(&self) -> &CStr {
        c"counting-collector"
    }

    fn add(
        &mut self,
        _key: &[u8],
        _value: &[u8],
        _entry_type: DBEntryType,
        _sequence: u64,
        _file_size: u64,
    ) {
    }

    fn finish(&mut self) -> HashMap<Vec<u8>, Vec<u8>> {
        HashMap::from([(
            COLLECTOR_ID_PROPERTY_KEY.to_vec(),
            self.id.to_le_bytes().to_vec(),
        )])
    }
}

impl Drop for CountingCollector {
    fn drop(&mut self) {
        self.counts
            .collectors_dropped
            .fetch_add(1, Ordering::SeqCst);
    }
}

struct CountingFactory {
    counts: Arc<LifecycleCounts>,
}

impl TablePropertiesCollectorFactory for CountingFactory {
    type Collector = CountingCollector;

    fn create(&self, _context: TablePropertiesCollectorContext) -> Self::Collector {
        let id = self.counts.next_collector_id.fetch_add(1, Ordering::SeqCst);
        self.counts
            .collectors_created
            .fetch_add(1, Ordering::SeqCst);
        CountingCollector {
            id,
            counts: Arc::clone(&self.counts),
        }
    }

    fn name(&self) -> &CStr {
        c"counting-factory"
    }
}

impl Drop for CountingFactory {
    fn drop(&mut self) {
        self.counts.factories_dropped.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OverlapRole {
    Flush,
    Compaction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GatedCreate {
    role: OverlapRole,
    context: TablePropertiesCollectorContext,
    collector_id: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct OverlapGateSnapshot {
    released: bool,
    timed_out: bool,
    in_flight: usize,
    max_in_flight: usize,
    flush: Option<GatedCreate>,
    compaction: Option<GatedCreate>,
    all_creates: Vec<(TablePropertiesCollectorContext, u64)>,
}

#[derive(Default)]
struct OverlapGateState {
    armed: bool,
    snapshot: OverlapGateSnapshot,
}

#[derive(Default)]
struct OverlapGate {
    state: Mutex<OverlapGateState>,
    changed: Condvar,
}

impl OverlapGate {
    fn arm(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(!state.armed, "overlap gate must only be armed once");
        state.armed = true;
    }

    fn enter(
        &self,
        context: TablePropertiesCollectorContext,
        collector_id: u64,
        timeout: Duration,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !state.armed || state.snapshot.released {
            return;
        }
        state.snapshot.all_creates.push((context, collector_id));
        // This classification is valid only for the controlled two-level,
        // leveled fixture below. The separate CFs, listener events, and final
        // LSM layout prove the sources; level 0 is not generally "a flush".
        let role = match context.level_at_creation {
            0 => OverlapRole::Flush,
            1 => OverlapRole::Compaction,
            _ => return,
        };

        let slot = match role {
            OverlapRole::Flush => &mut state.snapshot.flush,
            OverlapRole::Compaction => &mut state.snapshot.compaction,
        };
        if slot.is_some() {
            return;
        }
        *slot = Some(GatedCreate {
            role,
            context,
            collector_id,
        });
        state.snapshot.in_flight += 1;
        state.snapshot.max_in_flight = state.snapshot.max_in_flight.max(state.snapshot.in_flight);

        if state.snapshot.flush.is_some() && state.snapshot.compaction.is_some() {
            state.snapshot.released = true;
            self.changed.notify_all();
        } else {
            let (new_state, wait_result) = self
                .changed
                .wait_timeout_while(state, timeout, |state| !state.snapshot.released)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = new_state;
            if wait_result.timed_out() && !state.snapshot.released {
                state.snapshot.timed_out = true;
                state.snapshot.released = true;
                self.changed.notify_all();
            }
        }

        state.snapshot.in_flight -= 1;
        self.changed.notify_all();
    }

    fn force_release(&self) {
        let mut state = self.state.lock().expect("overlap gate lock poisoned");
        state.snapshot.released = true;
        self.changed.notify_all();
    }

    fn snapshot(&self) -> OverlapGateSnapshot {
        self.state
            .lock()
            .expect("overlap gate lock poisoned")
            .snapshot
            .clone()
    }
}

struct OverlapFactory {
    counts: Arc<LifecycleCounts>,
    gate: Arc<OverlapGate>,
    gate_timeout: Duration,
}

impl TablePropertiesCollectorFactory for OverlapFactory {
    type Collector = CountingCollector;

    fn create(&self, context: TablePropertiesCollectorContext) -> Self::Collector {
        let id = self.counts.next_collector_id.fetch_add(1, Ordering::SeqCst);
        self.counts
            .collectors_created
            .fetch_add(1, Ordering::SeqCst);
        self.gate.enter(context, id, self.gate_timeout);
        CountingCollector {
            id,
            counts: Arc::clone(&self.counts),
        }
    }

    fn name(&self) -> &CStr {
        c"overlap-factory"
    }
}

impl Drop for OverlapFactory {
    fn drop(&mut self) {
        self.counts.factories_dropped.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ObservedFlushEvent {
    cf_name: Option<Vec<u8>>,
    reason: DBFlushReason,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ObservedCompactionEvent {
    cf_name: Option<Vec<u8>>,
    reason: DBCompactionReason,
    status_ok: bool,
    base_input_level: i32,
    output_level: i32,
    input_file_count: usize,
    output_file_count: usize,
}

#[derive(Default)]
struct OverlapEvents {
    flushes: Mutex<Vec<ObservedFlushEvent>>,
    compactions: Mutex<Vec<ObservedCompactionEvent>>,
}

struct OverlapEventListener {
    events: Arc<OverlapEvents>,
}

impl EventListener for OverlapEventListener {
    fn on_flush_completed(&self, info: &FlushJobInfo) {
        if let Ok(mut flushes) = self.events.flushes.lock() {
            flushes.push(ObservedFlushEvent {
                cf_name: info.cf_name(),
                reason: info.flush_reason(),
            });
        }
    }

    fn on_compaction_completed(&self, info: &CompactionJobInfo) {
        if let Ok(mut compactions) = self.events.compactions.lock() {
            compactions.push(ObservedCompactionEvent {
                cf_name: info.cf_name(),
                reason: info.compaction_reason(),
                status_ok: info.status().is_ok(),
                base_input_level: info.base_input_level(),
                output_level: info.output_level(),
                input_file_count: info.input_file_count(),
                output_file_count: info.output_file_count(),
            });
        }
    }
}

#[derive(Default)]
struct UnsupportedFactoryObservations {
    name_calls: AtomicUsize,
    drops: AtomicUsize,
}

struct UnsupportedFactory {
    observations: Arc<UnsupportedFactoryObservations>,
}

impl TablePropertiesCollectorFactory for UnsupportedFactory {
    type Collector = ProbeCollector;

    fn create(&self, _context: TablePropertiesCollectorContext) -> Self::Collector {
        ProbeCollector
    }

    fn name(&self) -> &CStr {
        self.observations.name_calls.fetch_add(1, Ordering::SeqCst);
        c"unsupported-factory"
    }
}

impl Drop for UnsupportedFactory {
    fn drop(&mut self) {
        self.observations.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn concat_merge(
    _key: &[u8],
    existing_value: Option<&[u8]>,
    operands: &MergeOperands,
) -> Option<Vec<u8>> {
    let mut result = existing_value.unwrap_or_default().to_vec();
    for operand in operands {
        result.extend_from_slice(operand);
    }
    Some(result)
}

fn collector_id_from_default_cf(db: &DB) -> u64 {
    let collection = db
        .get_properties_of_all_tables()
        .expect("read default column family table properties");
    collector_id_from_collection(collection)
}

fn collector_id_from_named_cf(db: &DB, name: &str) -> u64 {
    let cf = db.cf_handle(name).expect("get named column family handle");
    let collection = db
        .get_properties_of_all_tables_cf(&cf)
        .expect("read named column family table properties");
    collector_id_from_collection(collection)
}

fn collector_id_from_collection(collection: rust_rocksdb::TablePropertiesCollection) -> u64 {
    let ids = collection
        .iter()
        .filter_map(|(_, properties)| {
            properties
                .user_collected_properties()
                .remove(COLLECTOR_ID_PROPERTY_KEY)
        })
        .collect::<Vec<_>>();

    assert_eq!(ids.len(), 1, "each column family should produce one SST");
    u64::from_le_bytes(
        ids[0]
            .as_slice()
            .try_into()
            .expect("collector id property must contain eight bytes"),
    )
}

fn collector_ids_from_named_cf(db: &DB, name: &str) -> Vec<u64> {
    let cf = db.cf_handle(name).expect("get named column family handle");
    let collection = db
        .get_properties_of_all_tables_cf(&cf)
        .expect("read named column family table properties");
    let mut ids = collection
        .iter()
        .filter_map(|(_, properties)| {
            properties
                .user_collected_properties()
                .remove(COLLECTOR_ID_PROPERTY_KEY)
        })
        .map(|id| {
            u64::from_le_bytes(
                id.as_slice()
                    .try_into()
                    .expect("collector id property must contain eight bytes"),
            )
        })
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids
}

fn num_files_at_level_cf(db: &DB, name: &str, level: usize) -> u64 {
    let cf = db.cf_handle(name).expect("get named column family handle");
    db.property_int_value_cf(&cf, num_files_at_level(level))
        .expect("read level file-count property")
        .expect("level file-count property must be present")
}

fn num_active_memtable_entries_cf(db: &DB, name: &str) -> u64 {
    let cf = db.cf_handle(name).expect("get named column family handle");
    db.property_int_value_cf(&cf, NUM_ENTRIES_ACTIVE_MEM_TABLE)
        .expect("read active memtable entry-count property")
        .expect("active memtable entry-count property must be present")
}

struct ProbeCollector;

impl TablePropertiesCollector for ProbeCollector {
    fn name(&self) -> &CStr {
        c"probe-collector"
    }

    fn add(
        &mut self,
        _key: &[u8],
        _value: &[u8],
        _entry_type: DBEntryType,
        _sequence: u64,
        _file_size: u64,
    ) {
    }

    fn finish(&mut self) -> HashMap<Vec<u8>, Vec<u8>> {
        HashMap::new()
    }
}

struct ProbeFactory;

impl TablePropertiesCollectorFactory for ProbeFactory {
    type Collector = ProbeCollector;

    fn create(&self, _context: TablePropertiesCollectorContext) -> Self::Collector {
        ProbeCollector
    }

    fn name(&self) -> &CStr {
        c"probe-factory"
    }
}

struct SequenceCollector {
    largest_sequence: u64,
}

impl TablePropertiesCollector for SequenceCollector {
    fn name(&self) -> &CStr {
        c"sequence-collector"
    }

    fn add(
        &mut self,
        _key: &[u8],
        _value: &[u8],
        _entry_type: DBEntryType,
        sequence: u64,
        _file_size: u64,
    ) {
        self.largest_sequence = self.largest_sequence.max(sequence);
    }

    fn finish(&mut self) -> HashMap<Vec<u8>, Vec<u8>> {
        HashMap::from([(
            KIWI_PROPERTY_KEY.to_vec(),
            format!("17/{}", self.largest_sequence).into_bytes(),
        )])
    }
}

struct SequenceFactory;

impl TablePropertiesCollectorFactory for SequenceFactory {
    type Collector = SequenceCollector;

    fn create(&self, _context: TablePropertiesCollectorContext) -> Self::Collector {
        SequenceCollector {
            largest_sequence: 0,
        }
    }

    fn name(&self) -> &CStr {
        c"sequence-factory"
    }
}

#[derive(Clone, Copy)]
enum PanicCase {
    FactoryCreate,
    CollectorName,
    Add,
    Finish,
}

impl PanicCase {
    fn from_env(value: &str) -> Self {
        match value {
            "factory_create" => Self::FactoryCreate,
            "collector_name" => Self::CollectorName,
            "add" => Self::Add,
            "finish" => Self::Finish,
            other => panic!("unknown collector panic case: {other}"),
        }
    }
}

struct PanickingCollector {
    case: PanicCase,
    largest_sequence: u64,
}

impl TablePropertiesCollector for PanickingCollector {
    fn name(&self) -> &CStr {
        if matches!(self.case, PanicCase::CollectorName) {
            panic!("collector name panic");
        }
        c"panicking-collector"
    }

    fn add(
        &mut self,
        _key: &[u8],
        _value: &[u8],
        _entry_type: DBEntryType,
        sequence: u64,
        _file_size: u64,
    ) {
        if matches!(self.case, PanicCase::Add) {
            panic!("collector add panic");
        }
        self.largest_sequence = self.largest_sequence.max(sequence);
    }

    fn finish(&mut self) -> HashMap<Vec<u8>, Vec<u8>> {
        if matches!(self.case, PanicCase::Finish) {
            panic!("collector finish panic");
        }
        HashMap::from([(
            KIWI_PROPERTY_KEY.to_vec(),
            format!("17/{}", self.largest_sequence).into_bytes(),
        )])
    }
}

struct PanickingFactory {
    case: PanicCase,
}

impl TablePropertiesCollectorFactory for PanickingFactory {
    type Collector = PanickingCollector;

    fn create(&self, _context: TablePropertiesCollectorContext) -> Self::Collector {
        if matches!(self.case, PanicCase::FactoryCreate) {
            panic!("collector factory create panic");
        }
        PanickingCollector {
            case: self.case,
            largest_sequence: 0,
        }
    }

    fn name(&self) -> &CStr {
        c"panicking-factory"
    }
}

struct ReadablePanickingCollector {
    largest_sequence: u64,
    readable_calls: Arc<AtomicUsize>,
}

impl TablePropertiesCollector for ReadablePanickingCollector {
    fn name(&self) -> &CStr {
        c"readable-panicking-collector"
    }

    fn add(
        &mut self,
        _key: &[u8],
        _value: &[u8],
        _entry_type: DBEntryType,
        sequence: u64,
        _file_size: u64,
    ) {
        self.largest_sequence = self.largest_sequence.max(sequence);
    }

    fn finish(&mut self) -> HashMap<Vec<u8>, Vec<u8>> {
        HashMap::from([(
            KIWI_PROPERTY_KEY.to_vec(),
            format!("17/{}", self.largest_sequence).into_bytes(),
        )])
    }

    fn get_readable_properties(&self) -> HashMap<Vec<u8>, Vec<u8>> {
        self.readable_calls.fetch_add(1, Ordering::SeqCst);
        panic!("readable properties panic");
    }
}

struct ReadablePanickingFactory {
    readable_calls: Arc<AtomicUsize>,
}

impl TablePropertiesCollectorFactory for ReadablePanickingFactory {
    type Collector = ReadablePanickingCollector;

    fn create(&self, _context: TablePropertiesCollectorContext) -> Self::Collector {
        ReadablePanickingCollector {
            largest_sequence: 0,
            readable_calls: Arc::clone(&self.readable_calls),
        }
    }

    fn name(&self) -> &CStr {
        c"readable-panicking-factory"
    }
}

struct DropPanickingFactory {
    drops: Arc<AtomicUsize>,
}

impl TablePropertiesCollectorFactory for DropPanickingFactory {
    type Collector = ProbeCollector;

    fn create(&self, _context: TablePropertiesCollectorContext) -> Self::Collector {
        ProbeCollector
    }

    fn name(&self) -> &CStr {
        c"drop-panicking-factory"
    }
}

impl Drop for DropPanickingFactory {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        panic!("factory drop panic");
    }
}

struct DropPanickingCollector {
    drops: Arc<AtomicUsize>,
}

impl TablePropertiesCollector for DropPanickingCollector {
    fn name(&self) -> &CStr {
        c"drop-panicking-collector"
    }

    fn add(
        &mut self,
        _key: &[u8],
        _value: &[u8],
        _entry_type: DBEntryType,
        _sequence: u64,
        _file_size: u64,
    ) {
    }

    fn finish(&mut self) -> HashMap<Vec<u8>, Vec<u8>> {
        HashMap::from([(KIWI_PROPERTY_KEY.to_vec(), b"17/1".to_vec())])
    }
}

impl Drop for DropPanickingCollector {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        panic!("collector drop panic");
    }
}

struct DropPanickingCollectorFactory {
    drops: Arc<AtomicUsize>,
}

impl TablePropertiesCollectorFactory for DropPanickingCollectorFactory {
    type Collector = DropPanickingCollector;

    fn create(&self, _context: TablePropertiesCollectorContext) -> Self::Collector {
        DropPanickingCollector {
            drops: Arc::clone(&self.drops),
        }
    }

    fn name(&self) -> &CStr {
        c"drop-panicking-collector-factory"
    }
}

fn prepare_fail_fast_database(path: &Path) {
    let mut options = Options::default();
    options.create_if_missing(true);
    options.set_table_properties_collector_factory(SequenceFactory);
    let db = DB::open(&options, path).expect("create fail-fast test database");
    db.put(b"baseline-key", b"baseline-value")
        .expect("write fail-fast baseline value");
    db.flush().expect("flush fail-fast baseline SST");
}

fn assert_all_installed_ssts_have_kiwi_property(path: &Path) {
    let options = Options::default();
    let db = DB::open_for_read_only(&options, path, false)
        .expect("reopen database read-only after collector child abort");
    let collection = db
        .get_properties_of_all_tables()
        .expect("read table properties after collector child abort");
    assert!(
        !collection.is_empty(),
        "the prepared baseline SST must remain installed"
    );
    for (file_name, properties) in collection.iter() {
        assert!(
            properties
                .user_collected_properties()
                .contains_key(KIWI_PROPERTY_KEY),
            "installed SST {file_name:?} is missing the Kiwi recovery property"
        );
    }
}

fn assert_collector_panic_fails_fast(case: &str) {
    if skip_bundled_backend_only_test() {
        return;
    }

    let path = DBPath::new("_rust_rocksdb_collector_fail_fast");
    let path_ref = &path;
    let db_path: &Path = path_ref.as_ref();
    prepare_fail_fast_database(db_path);

    let output = Command::new(env::current_exe().expect("locate current test executable"))
        .args(["--exact", "collector_subprocess_entry", "--nocapture"])
        .env(COLLECTOR_PANIC_CASE_ENV, case)
        .env(COLLECTOR_PANIC_DB_ENV, db_path)
        .output()
        .expect("run collector panic child process");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "collector panic child unexpectedly succeeded; stdout={stdout}; stderr={stderr}"
    );
    assert!(
        stdout.contains(CHILD_STARTED_MARKER),
        "collector panic child did not reach its marked entry point; stdout={stdout}; stderr={stderr}"
    );
    assert!(
        !stdout.contains(FLUSH_SUCCEEDED_MARKER),
        "flush returned successfully after a critical collector panic; stdout={stdout}; stderr={stderr}"
    );
    let expected_diagnostic = match case {
        "factory_create" | "collector_name" => {
            "rust-rocksdb: table properties collector factory callback failed"
        }
        "add" => "rust-rocksdb: table properties collector add callback failed",
        "finish" => "rust-rocksdb: table properties collector finish callback failed",
        _ => unreachable!("test cases are validated before spawning the child"),
    };
    assert!(
        stderr.contains(expected_diagnostic),
        "collector panic child did not reach the expected fail-fast boundary; expected={expected_diagnostic}; stdout={stdout}; stderr={stderr}"
    );

    assert_all_installed_ssts_have_kiwi_property(db_path);
}

fn assert_collector_contract<T: TablePropertiesCollector + Send + 'static>() {}
fn assert_factory_contract<T: TablePropertiesCollectorFactory + Send + Sync + 'static>() {}

fn collector_factory_is_supported() -> bool {
    match unsafe {
        rust_librocksdb_sys::rust_rocksdb_table_properties_collector_factory_supported()
    } {
        0 => false,
        1 => true,
        other => panic!("unexpected collector factory capability value: {other}"),
    }
}

fn skip_bundled_backend_only_test() -> bool {
    let skip = !collector_factory_is_supported();
    if skip {
        eprintln!("skipping bundled-backend-only collector factory test");
    }
    skip
}

#[derive(Debug)]
struct ChildWatchdogResult {
    output: Output,
    timed_out: bool,
    poll_error: Option<io::Error>,
}

#[derive(Debug)]
struct ChildWatchdogError {
    timed_out: bool,
    poll_error: Option<io::Error>,
    operation: &'static str,
    operation_error: io::Error,
}

impl std::fmt::Display for ChildWatchdogError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "child watchdog {} failed: {}; timed_out={}; poll_error={:?}",
            self.operation, self.operation_error, self.timed_out, self.poll_error
        )
    }
}

impl std::error::Error for ChildWatchdogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.operation_error)
    }
}

static CHILD_REAPER_SENDER: OnceLock<Result<mpsc::Sender<Child>, io::Error>> = OnceLock::new();

fn child_reaper_sender() -> Result<&'static mpsc::Sender<Child>, &'static io::Error> {
    match CHILD_REAPER_SENDER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<Child>();
        thread::Builder::new()
            .name("rust-rocksdb-test-child-reaper".to_owned())
            .spawn(move || {
                while let Ok(child) = receiver.recv() {
                    if let Err(error) = child.wait_with_output() {
                        eprintln!("async child reaper failed to wait for child output: {error}");
                    }
                }
            })
            .map(|_| sender)
    }) {
        Ok(sender) => Ok(sender),
        Err(error) => Err(error),
    }
}

fn submit_child_to_reaper(sender: &mpsc::Sender<Child>, child: Child) {
    if let Err(error) = sender.send(child) {
        // The sender is stored in a process-lifetime OnceLock and the worker
        // handles wait failures without unwinding, so disconnect is unreachable.
        // Avoid dropping an unreaped child even if that invariant is violated.
        std::mem::forget(error.0);
        panic!("fixed child reaper unexpectedly disconnected");
    }
}

fn wait_for_child_with_watchdog<F>(
    child: Child,
    timeout: Duration,
    poll: F,
    child_reaper: &'static mpsc::Sender<Child>,
) -> io::Result<ChildWatchdogResult>
where
    F: FnMut(&mut Child) -> io::Result<Option<ExitStatus>>,
{
    wait_for_child_with_watchdog_ops(
        child,
        timeout,
        poll,
        Child::kill,
        Child::wait_with_output,
        move |child| submit_child_to_reaper(child_reaper, child),
    )
    .map_err(io::Error::other)
}

fn wait_for_child_with_watchdog_ops<C, P, K, W, R>(
    mut child: C,
    timeout: Duration,
    mut poll: P,
    kill: K,
    wait_with_output: W,
    submit_reaper: R,
) -> Result<ChildWatchdogResult, ChildWatchdogError>
where
    P: FnMut(&mut C) -> io::Result<Option<ExitStatus>>,
    K: FnOnce(&mut C) -> io::Result<()>,
    W: FnOnce(C) -> io::Result<Output>,
    R: FnOnce(C),
{
    let deadline = Instant::now() + timeout;
    let (timed_out, poll_error) = loop {
        match poll(&mut child) {
            Ok(Some(_)) => break (false, None),
            Ok(None) if Instant::now() >= deadline => break (true, None),
            Ok(None) => {
                // This sleep only paces the watchdog; child synchronization is
                // controlled by explicit markers and blocking primitives.
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => break (false, Some(error)),
        }
    };
    if (timed_out || poll_error.is_some())
        && let Err(operation_error) = kill(&mut child)
        && !matches!(poll(&mut child), Ok(Some(_)))
    {
        submit_reaper(child);
        return Err(ChildWatchdogError {
            timed_out,
            poll_error,
            operation: "kill",
            operation_error,
        });
    }
    let output = match wait_with_output(child) {
        Ok(output) => output,
        Err(operation_error) => {
            return Err(ChildWatchdogError {
                timed_out,
                poll_error,
                operation: "wait_with_output",
                operation_error,
            });
        }
    };
    Ok(ChildWatchdogResult {
        output,
        timed_out,
        poll_error,
    })
}

#[cfg(unix)]
fn successful_watchdog_test_output() -> Output {
    use std::os::unix::process::ExitStatusExt;

    Output {
        status: ExitStatus::from_raw(0),
        stdout: Vec::new(),
        stderr: Vec::new(),
    }
}

#[cfg(windows)]
fn successful_watchdog_test_output() -> Output {
    use std::os::windows::process::ExitStatusExt;

    Output {
        status: ExitStatus::from_raw(0),
        stdout: Vec::new(),
        stderr: Vec::new(),
    }
}

#[test]
fn child_watchdog_poll_error_kills_and_waits_for_child() {
    let kill_calls = Arc::new(AtomicUsize::new(0));
    let observed_kill_calls = Arc::clone(&kill_calls);
    let wait_calls = Arc::new(AtomicUsize::new(0));
    let observed_wait_calls = Arc::clone(&wait_calls);

    let watchdog = wait_for_child_with_watchdog_ops(
        (),
        Duration::from_secs(1),
        |_| Err::<Option<ExitStatus>, _>(io::Error::other(INJECTED_POLL_ERROR)),
        move |_| {
            observed_kill_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
        move |_| {
            observed_wait_calls.fetch_add(1, Ordering::SeqCst);
            Ok(successful_watchdog_test_output())
        },
        |_| panic!("reaper must not run after a successful kill"),
    )
    .expect("poll failure with successful kill must reap the child");

    assert_eq!(kill_calls.load(Ordering::SeqCst), 1);
    assert_eq!(wait_calls.load(Ordering::SeqCst), 1);
    assert!(!watchdog.timed_out, "{watchdog:?}");
    assert_eq!(
        watchdog.poll_error.as_ref().map(io::Error::to_string),
        Some(INJECTED_POLL_ERROR.to_owned())
    );
}

#[test]
fn child_watchdog_timeout_kills_and_waits_for_child() {
    let kill_calls = Arc::new(AtomicUsize::new(0));
    let observed_kill_calls = Arc::clone(&kill_calls);
    let wait_calls = Arc::new(AtomicUsize::new(0));
    let observed_wait_calls = Arc::clone(&wait_calls);

    let watchdog = wait_for_child_with_watchdog_ops(
        (),
        Duration::ZERO,
        |_| Ok(None),
        move |_| {
            observed_kill_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
        move |_| {
            observed_wait_calls.fetch_add(1, Ordering::SeqCst);
            Ok(successful_watchdog_test_output())
        },
        |_| panic!("reaper must not run after a successful kill"),
    )
    .expect("timeout with successful kill must reap the child");

    assert_eq!(kill_calls.load(Ordering::SeqCst), 1);
    assert_eq!(wait_calls.load(Ordering::SeqCst), 1);
    assert!(watchdog.timed_out, "{watchdog:?}");
    assert!(watchdog.poll_error.is_none(), "{watchdog:?}");
}

#[test]
fn child_watchdog_kill_error_does_not_enter_wait_path() {
    let wait_calls = Arc::new(AtomicUsize::new(0));
    let observed_wait_calls = Arc::clone(&wait_calls);
    let reaper_calls = Arc::new(AtomicUsize::new(0));
    let observed_reaper_calls = Arc::clone(&reaper_calls);

    let error = wait_for_child_with_watchdog_ops(
        (),
        Duration::ZERO,
        |_| {
            Err::<Option<ExitStatus>, _>(io::Error::new(
                io::ErrorKind::TimedOut,
                INJECTED_POLL_ERROR,
            ))
        },
        |_| Err(io::Error::from_raw_os_error(5)),
        move |_| {
            observed_wait_calls.fetch_add(1, Ordering::SeqCst);
            Err::<Output, _>(io::Error::other("wait must not be called after kill fails"))
        },
        move |_| {
            observed_reaper_calls.fetch_add(1, Ordering::SeqCst);
        },
    )
    .expect_err("kill failure must terminate the watchdog without waiting");

    assert_eq!(wait_calls.load(Ordering::SeqCst), 0);
    assert_eq!(reaper_calls.load(Ordering::SeqCst), 1);
    assert!(!error.timed_out, "{error}");
    let poll_error = error
        .poll_error
        .as_ref()
        .expect("kill error must retain the original poll error");
    assert_eq!(poll_error.kind(), io::ErrorKind::TimedOut);
    assert_eq!(poll_error.to_string(), INJECTED_POLL_ERROR);
    assert_eq!(error.operation, "kill");
    assert_eq!(error.operation_error.raw_os_error(), Some(5));
    let source = std::error::Error::source(&error)
        .and_then(|source| source.downcast_ref::<io::Error>())
        .expect("watchdog error source must be the original kill error");
    assert_eq!(source.raw_os_error(), Some(5));
}

#[test]
fn child_watchdog_kill_error_after_child_exit_waits_for_output() {
    let poll_calls = Arc::new(AtomicUsize::new(0));
    let observed_poll_calls = Arc::clone(&poll_calls);
    let wait_calls = Arc::new(AtomicUsize::new(0));
    let observed_wait_calls = Arc::clone(&wait_calls);

    let watchdog = wait_for_child_with_watchdog_ops(
        (),
        Duration::ZERO,
        move |_| match observed_poll_calls.fetch_add(1, Ordering::SeqCst) {
            0 => Ok(None),
            1 => Ok(Some(successful_watchdog_test_output().status)),
            call => panic!("watchdog unexpectedly polled child a third time: {call}"),
        },
        |_| Err(io::Error::from_raw_os_error(5)),
        move |_| {
            observed_wait_calls.fetch_add(1, Ordering::SeqCst);
            Ok(successful_watchdog_test_output())
        },
        |_| panic!("reaper must not run after the child has already exited"),
    )
    .expect("kill error after child exit must still collect its output");

    assert_eq!(poll_calls.load(Ordering::SeqCst), 2);
    assert_eq!(wait_calls.load(Ordering::SeqCst), 1);
    assert!(watchdog.timed_out, "{watchdog:?}");
    assert!(watchdog.poll_error.is_none(), "{watchdog:?}");
    assert!(watchdog.output.status.success(), "{watchdog:?}");
}

#[test]
fn child_watchdog_wait_error_returns_with_poll_context() {
    let kill_calls = Arc::new(AtomicUsize::new(0));
    let observed_kill_calls = Arc::clone(&kill_calls);
    let wait_calls = Arc::new(AtomicUsize::new(0));
    let observed_wait_calls = Arc::clone(&wait_calls);

    let error = wait_for_child_with_watchdog_ops(
        (),
        Duration::from_secs(1),
        |_| Err::<Option<ExitStatus>, _>(io::Error::other(INJECTED_POLL_ERROR)),
        move |_| {
            observed_kill_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
        move |_| {
            observed_wait_calls.fetch_add(1, Ordering::SeqCst);
            Err(io::Error::other("injected child wait error"))
        },
        |_| panic!("reaper must not run after a successful kill"),
    )
    .expect_err("wait failure must return instead of losing watchdog context");

    assert_eq!(kill_calls.load(Ordering::SeqCst), 1);
    assert_eq!(wait_calls.load(Ordering::SeqCst), 1);
    assert!(!error.timed_out, "{error}");
    assert_eq!(
        error.poll_error.as_ref().map(io::Error::to_string),
        Some(INJECTED_POLL_ERROR.to_owned())
    );
    assert_eq!(error.operation, "wait_with_output");
    assert_eq!(error.operation_error.kind(), io::ErrorKind::Other);
    assert_eq!(
        error.operation_error.to_string(),
        "injected child wait error"
    );
    let source = std::error::Error::source(&error)
        .and_then(|source| source.downcast_ref::<io::Error>())
        .expect("watchdog error source must be the original wait error");
    assert_eq!(source.kind(), io::ErrorKind::Other);
}

#[test]
fn child_watchdog_poll_error_subprocess_entry() {
    if env::var_os(WATCHDOG_POLL_ERROR_CHILD_ENV).is_none() {
        return;
    }
    let started_path = env::var_os(WATCHDOG_POLL_ERROR_STARTED_PATH_ENV)
        .expect("watchdog poll-error child start-marker path must be provided");
    fs::write(&started_path, WATCHDOG_POLL_ERROR_PARTIAL_MARKER)
        .expect("write partial watchdog poll-error child start marker");

    let mut stdin = io::stdin().lock();
    let mut complete_marker = [0_u8; 1];
    stdin
        .read_exact(&mut complete_marker)
        .expect("wait for watchdog poll-error marker completion signal");
    fs::write(&started_path, WATCHDOG_POLL_ERROR_STARTED_MARKER)
        .expect("complete watchdog poll-error child start marker");

    let mut buffer = [0_u8; 1];
    loop {
        match stdin.read(&mut buffer) {
            Ok(0) => return,
            Ok(_) => {}
            Err(error) => panic!("read watchdog poll-error child stdin: {error}"),
        }
    }
}

#[test]
fn child_watchdog_reaps_running_child_before_reporting_poll_error() {
    let marker_dir = tempfile::tempdir().expect("create watchdog poll-error marker directory");
    let started_path = marker_dir.path().join("child-started");
    let child_reaper =
        child_reaper_sender().expect("start child reaper before spawning watchdog child process");
    let mut child = Command::new(env::current_exe().expect("locate current test executable"))
        .args([
            "--exact",
            "child_watchdog_poll_error_subprocess_entry",
            "--nocapture",
        ])
        .env(WATCHDOG_POLL_ERROR_CHILD_ENV, "1")
        .env(WATCHDOG_POLL_ERROR_STARTED_PATH_ENV, &started_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn watchdog poll-error child process");
    let mut child_stdin = child
        .stdin
        .take()
        .expect("open watchdog poll-error child stdin");
    let injections = Arc::new(AtomicUsize::new(0));
    let poll_injections = Arc::clone(&injections);
    let marker_completions = Arc::new(AtomicUsize::new(0));
    let poll_marker_completions = Arc::clone(&marker_completions);
    let poll_started_path = started_path.clone();

    let watchdog = wait_for_child_with_watchdog(
        child,
        Duration::from_secs(15),
        move |child| {
            if fs::read_to_string(&poll_started_path)
                .is_ok_and(|marker| marker == WATCHDOG_POLL_ERROR_STARTED_MARKER)
                && poll_injections.load(Ordering::SeqCst) == 0
            {
                let status = child.try_wait()?;
                assert!(
                    status.is_none(),
                    "watchdog poll-error child exited before error injection: {status:?}"
                );
                poll_injections.fetch_add(1, Ordering::SeqCst);
                return Err(io::Error::other(INJECTED_POLL_ERROR));
            }
            if fs::read_to_string(&poll_started_path)
                .is_ok_and(|marker| marker == WATCHDOG_POLL_ERROR_PARTIAL_MARKER)
                && poll_marker_completions.load(Ordering::SeqCst) == 0
            {
                child_stdin.write_all(b"1")?;
                child_stdin.flush()?;
                poll_marker_completions.fetch_add(1, Ordering::SeqCst);
            }
            child.try_wait()
        },
        child_reaper,
    )
    .expect("watchdog must reap the child before returning the poll error");

    assert_eq!(injections.load(Ordering::SeqCst), 1);
    assert_eq!(
        marker_completions.load(Ordering::SeqCst),
        1,
        "the watchdog must not inject a poll error before the marker is complete"
    );
    assert!(!watchdog.timed_out, "{watchdog:?}");
    assert_eq!(
        watchdog.poll_error.as_ref().map(io::Error::to_string),
        Some(INJECTED_POLL_ERROR.to_owned())
    );
    assert!(
        !watchdog.output.status.success(),
        "poll-error child was not terminated: {watchdog:?}"
    );
    assert_eq!(
        fs::read_to_string(&started_path).expect("read watchdog child start marker"),
        WATCHDOG_POLL_ERROR_STARTED_MARKER
    );
}

#[test]
fn collector_subprocess_entry() {
    let Ok(case) = env::var(COLLECTOR_PANIC_CASE_ENV) else {
        return;
    };
    if skip_bundled_backend_only_test() {
        return;
    }
    let db_path = env::var_os(COLLECTOR_PANIC_DB_ENV)
        .expect("collector panic child database path must be provided");

    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{CHILD_STARTED_MARKER}").expect("write child start marker");
    stdout.flush().expect("flush child start marker");
    drop(stdout);

    let mut options = Options::default();
    options.set_table_properties_collector_factory(PanickingFactory {
        case: PanicCase::from_env(&case),
    });
    let db = DB::open(&options, &db_path).expect("open collector panic child database");
    db.put(b"panic-key", b"panic-value")
        .expect("write collector panic child value");
    db.flush()
        .expect("critical collector panic must not return from flush");

    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{FLUSH_SUCCEEDED_MARKER}").expect("write flush success marker");
    stdout.flush().expect("flush success marker");
}

#[test]
fn fail_fast_factory_create_panic_aborts_before_installing_an_unprotected_sst() {
    assert_collector_panic_fails_fast("factory_create");
}

#[test]
fn fail_fast_collector_name_panic_aborts_before_installing_an_unprotected_sst() {
    assert_collector_panic_fails_fast("collector_name");
}

#[test]
fn fail_fast_collector_add_panic_aborts_before_installing_an_unprotected_sst() {
    assert_collector_panic_fails_fast("add");
}

#[test]
fn fail_fast_collector_finish_panic_aborts_before_installing_an_unprotected_sst() {
    assert_collector_panic_fails_fast("finish");
}

#[test]
fn readable_panic_keeps_binary_property_and_degrades_readable_properties_to_empty() {
    if skip_bundled_backend_only_test() {
        return;
    }

    let path = DBPath::new("_rust_rocksdb_collector_readable_panic");
    let readable_calls = Arc::new(AtomicUsize::new(0));
    let mut options = Options::default();
    options.create_if_missing(true);
    options.set_table_properties_collector_factory(ReadablePanickingFactory {
        readable_calls: Arc::clone(&readable_calls),
    });
    let db = DB::open(&options, &path).expect("open readable panic test database");
    db.put(b"key", b"value")
        .expect("write readable panic test value");
    db.flush().expect("flush despite readable properties panic");

    let collection = db
        .get_properties_of_all_tables()
        .expect("read properties after readable callback panic");
    assert_eq!(collection.len(), 1);
    let (_, properties) = collection
        .iter()
        .next()
        .expect("readable panic test must install one SST");
    assert_eq!(
        properties.user_collected_properties()[KIWI_PROPERTY_KEY],
        b"17/1"
    );
    assert!(
        readable_calls.load(Ordering::SeqCst) > 0,
        "RocksDB must execute the injected readable-properties panic path"
    );
    assert!(properties.readable_properties().is_empty());
}

#[test]
fn drop_panic_factory_is_caught_and_factory_is_dropped_once() {
    if skip_bundled_backend_only_test() {
        return;
    }

    let drops = Arc::new(AtomicUsize::new(0));
    let mut options = Options::default();
    options.set_table_properties_collector_factory(DropPanickingFactory {
        drops: Arc::clone(&drops),
    });

    drop(options);

    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn drop_panic_collector_is_caught_and_collector_is_dropped_once() {
    if skip_bundled_backend_only_test() {
        return;
    }

    let path = DBPath::new("_rust_rocksdb_collector_drop_panic");
    let drops = Arc::new(AtomicUsize::new(0));
    let mut options = Options::default();
    options.create_if_missing(true);
    options.set_table_properties_collector_factory(DropPanickingCollectorFactory {
        drops: Arc::clone(&drops),
    });
    let db = DB::open(&options, &path).expect("open collector drop panic test database");
    db.put(b"key", b"value")
        .expect("write collector drop panic test value");

    db.flush().expect("flush despite collector drop panic");

    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn collector_and_factory_have_the_required_thread_contracts() {
    assert_collector_contract::<ProbeCollector>();
    assert_factory_contract::<ProbeFactory>();
}

#[test]
fn collector_context_preserves_every_rocksdb_field() {
    let context = TablePropertiesCollectorContext {
        column_family_id: 7,
        level_at_creation: 3,
        num_levels: 8,
        last_level_inclusive_max_seqno_threshold: 42,
    };

    assert_eq!(context.column_family_id, 7);
    assert_eq!(context.level_at_creation, 3);
    assert_eq!(context.num_levels, 8);
    assert_eq!(context.last_level_inclusive_max_seqno_threshold, 42);
}

#[test]
fn db_entry_type_maps_known_and_unknown_rocksdb_values() {
    let known = [
        DBEntryType::Put,
        DBEntryType::Delete,
        DBEntryType::SingleDelete,
        DBEntryType::Merge,
        DBEntryType::RangeDeletion,
        DBEntryType::BlobIndex,
        DBEntryType::DeleteWithTimestamp,
        DBEntryType::WideColumnEntity,
        DBEntryType::TimedPut,
    ];

    for (raw, expected) in known.into_iter().enumerate() {
        assert_eq!(expected as u8, raw as u8);
        assert_eq!(DBEntryType::from(raw as u8), expected);
    }
    assert_eq!(DBEntryType::Other as u8, 9);
    assert_eq!(DBEntryType::from(9), DBEntryType::Other);
    assert_eq!(DBEntryType::from(u8::MAX), DBEntryType::Other);
}

#[test]
fn readable_properties_default_to_empty() {
    assert!(ProbeCollector.get_readable_properties().is_empty());
}

#[test]
fn collector_factory_capability_matches_the_selected_backend() {
    let supported = collector_factory_is_supported();
    if let Ok(expected) = env::var(EXPECTED_COLLECTOR_FACTORY_SUPPORT_ENV) {
        let expected = match expected.as_str() {
            "0" => false,
            "1" => true,
            other => panic!("unexpected {EXPECTED_COLLECTOR_FACTORY_SUPPORT_ENV} value: {other}"),
        };
        assert_eq!(supported, expected);
    }

    match supported {
        true => {}
        false => {
            let observations = Arc::new(UnsupportedFactoryObservations::default());
            let mut options = Options::default();
            let result = catch_unwind(AssertUnwindSafe(|| {
                options.set_table_properties_collector_factory(UnsupportedFactory {
                    observations: Arc::clone(&observations),
                });
            }));

            let panic = result.expect_err("the system backend must fail closed");
            let message = panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&'static str>().copied())
                .expect("the fail-closed panic must contain a string message");
            assert!(
                message.contains(
                    "TablePropertiesCollectorFactory requires the bundled RocksDB backend"
                ),
                "unexpected fail-closed panic: {message}"
            );
            assert_eq!(observations.name_calls.load(Ordering::SeqCst), 0);
            assert_eq!(observations.drops.load(Ordering::SeqCst), 1);
        }
    }
}

#[test]
fn collector_factory_writes_kiwi_property_bytes_during_flush() {
    if skip_bundled_backend_only_test() {
        return;
    }

    let path = DBPath::new("_rust_rocksdb_table_properties_collector_factory");
    let mut options = Options::default();
    options.create_if_missing(true);
    options.set_table_properties_collector_factory(SequenceFactory);

    let db = DB::open(&options, &path).expect("open collector test database");
    db.put(b"key-1", b"value-1").expect("put key-1");
    db.put(b"key-2", b"value-2").expect("put key-2");
    db.flush().expect("flush collector test database");

    let collection = db
        .get_properties_of_all_tables()
        .expect("read collected table properties");
    let values = collection
        .iter()
        .filter_map(|(_, properties)| {
            properties
                .user_collected_properties()
                .remove(KIWI_PROPERTY_KEY)
        })
        .collect::<Vec<_>>();

    assert_eq!(values, vec![b"17/2".to_vec()]);
}

#[test]
fn context_real_flush_preserves_entry_arguments_and_all_context_fields() {
    if skip_bundled_backend_only_test() {
        return;
    }

    let path = DBPath::new("_rust_rocksdb_collector_context_arguments");
    let observations = Arc::new(CallbackObservations::default());
    let mut block_options = BlockBasedOptions::default();
    block_options.set_block_size(1);
    let mut options = Options::default();
    options.create_if_missing(true);
    options.set_block_based_table_factory(&block_options);
    options.set_merge_operator_associative("concat", concat_merge);
    options.set_table_properties_collector_factory(ObservingFactory {
        observations: Arc::clone(&observations),
    });

    let db = DB::open(&options, &path).expect("open callback argument test database");
    db.put(b"single-key", b"single-value")
        .expect("put value for later single delete");
    db.flush().expect("flush value for later single delete");
    observations
        .contexts
        .lock()
        .expect("observed contexts lock poisoned")
        .clear();
    observations
        .entries
        .lock()
        .expect("observed entries lock poisoned")
        .clear();

    db.put(b"put-key", b"put-value").expect("put value");
    db.delete(b"delete-key").expect("write delete tombstone");
    db.single_delete(b"single-key")
        .expect("write single-delete tombstone");
    db.merge(b"merge-key", b"merge-value")
        .expect("write merge operand");
    let mut batch = WriteBatch::default();
    batch.delete_range(b"range-a", b"range-z");
    db.write(&batch).expect("write range-delete tombstone");
    db.flush().expect("flush callback argument test database");

    let contexts = observations
        .contexts
        .lock()
        .expect("observed contexts lock poisoned")
        .clone();
    assert_eq!(
        contexts,
        vec![TablePropertiesCollectorContext {
            column_family_id: 0,
            level_at_creation: 0,
            num_levels: 7,
            last_level_inclusive_max_seqno_threshold: (1_u64 << 56) - 1,
        }]
    );

    let entries = observations
        .entries
        .lock()
        .expect("observed entries lock poisoned")
        .clone();
    let find = |key: &[u8], entry_type| {
        entries
            .iter()
            .find(|entry| entry.key == key && entry.entry_type == entry_type)
            .unwrap_or_else(|| panic!("missing {entry_type:?} callback for {key:?}"))
    };

    let delete = find(b"delete-key", DBEntryType::Delete);
    assert_eq!(delete.value, b"");
    assert_eq!(delete.sequence, 3);

    let merge = find(b"merge-key", DBEntryType::Merge);
    assert_eq!(merge.value, b"merge-value");
    assert_eq!(merge.sequence, 5);

    let put = find(b"put-key", DBEntryType::Put);
    assert_eq!(put.value, b"put-value");
    assert_eq!(put.sequence, 2);

    let range_delete = find(b"range-a", DBEntryType::RangeDeletion);
    assert_eq!(range_delete.value, b"range-z");
    assert_eq!(range_delete.sequence, 6);

    let single_delete = find(b"single-key", DBEntryType::SingleDelete);
    assert_eq!(single_delete.value, b"");
    assert_eq!(single_delete.sequence, 4);

    assert!(
        entries
            .windows(2)
            .all(|pair| pair[0].file_size <= pair[1].file_size),
        "file size should advance monotonically while RocksDB builds the SST: {entries:?}"
    );
    assert!(
        entries.iter().any(|entry| entry.file_size > 0),
        "the callback must receive RocksDB's live nonzero file-size estimate"
    );
}

#[test]
fn concurrent_multi_cf_flush_keeps_collector_properties_isolated() {
    if skip_bundled_backend_only_test() {
        return;
    }

    let path = DBPath::new("_rust_rocksdb_collector_concurrent_multi_cf");
    let counts = Arc::new(LifecycleCounts::default());
    let mut options = Options::default();
    options.create_if_missing(true);
    options.create_missing_column_families(true);
    options.set_max_background_jobs(4);
    options.set_table_properties_collector_factory(CountingFactory {
        counts: Arc::clone(&counts),
    });
    let descriptors = ["default", "cf1", "cf2"].map(|name| {
        let mut cf_options = options.clone();
        cf_options.set_max_background_jobs(4);
        ColumnFamilyDescriptor::new(name, cf_options)
    });
    let db = DB::open_cf_descriptors(&options, &path, descriptors)
        .expect("open multi-column-family collector database");

    db.put(b"default-key", b"default-value")
        .expect("put default column family value");
    let cf1 = db.cf_handle("cf1").expect("get cf1 handle");
    let cf2 = db.cf_handle("cf2").expect("get cf2 handle");
    db.put_cf(&cf1, b"cf1-key", b"cf1-value")
        .expect("put cf1 value");
    db.put_cf(&cf2, b"cf2-key", b"cf2-value")
        .expect("put cf2 value");

    let default_cf = db.cf_handle("default").expect("get default cf handle");
    let mut flush_options = FlushOptions::default();
    flush_options.set_wait(true);
    db.flush_cfs_opt(&[&default_cf, &cf1, &cf2], &flush_options)
        .expect("flush all column families");

    let default_id = collector_id_from_default_cf(&db);
    let cf1_id = collector_id_from_named_cf(&db, "cf1");
    let cf2_id = collector_id_from_named_cf(&db, "cf2");
    let mut ids = [default_id, cf1_id, cf2_id];
    ids.sort_unstable();

    assert_eq!(ids, [0, 1, 2]);
    assert_eq!(counts.collectors_created.load(Ordering::SeqCst), 3);
    assert_eq!(counts.collectors_dropped.load(Ordering::SeqCst), 3);
}

fn run_factory_create_overlap_scenario(db_path: &Path) {
    const COMPACT_CF: &str = "compact_cf";
    const FLUSH_CF: &str = "flush_cf";
    const GATE_TIMEOUT: Duration = Duration::from_secs(15);
    const OPERATION_TIMEOUT: Duration = Duration::from_secs(30);

    let counts = Arc::new(LifecycleCounts::default());
    let gate = Arc::new(OverlapGate::default());
    let events = Arc::new(OverlapEvents::default());
    let mut env = Env::new().expect("create isolated RocksDB environment");
    env.set_high_priority_background_threads(2);
    env.set_low_priority_background_threads(2);

    let mut options = Options::default();
    options.create_if_missing(true);
    options.create_missing_column_families(true);
    options.set_env(&env);
    options.set_max_background_jobs(4);
    options.set_max_subcompactions(1);
    options.set_num_levels(2);
    options.set_disable_auto_compactions(true);
    options.set_level_zero_file_num_compaction_trigger(1000);
    options.set_target_file_size_base(64 * 1024 * 1024);
    options.add_event_listener(OverlapEventListener {
        events: Arc::clone(&events),
    });
    options.set_table_properties_collector_factory(OverlapFactory {
        counts: Arc::clone(&counts),
        gate: Arc::clone(&gate),
        gate_timeout: GATE_TIMEOUT,
    });
    let descriptors = ["default", COMPACT_CF, FLUSH_CF]
        .map(|name| ColumnFamilyDescriptor::new(name, options.clone()));
    let db = Arc::new(
        DB::open_cf_descriptors(&options, db_path, descriptors)
            .expect("open flush-compaction overlap database"),
    );
    drop(options);

    let compact_value = vec![b'c'; 1024];
    for round in 0..3 {
        let compact_cf = db
            .cf_handle(COMPACT_CF)
            .expect("get compact column family handle");
        for key_index in 0..128 {
            let key = format!("overlap-key-{key_index:04}");
            let mut value = compact_value.clone();
            value.extend_from_slice(format!("-{round}").as_bytes());
            db.put_cf(&compact_cf, key.as_bytes(), value)
                .expect("write overlapping compaction input");
        }
        let mut flush_options = FlushOptions::default();
        flush_options.set_wait(true);
        db.flush_cf_opt(&compact_cf, &flush_options)
            .expect("flush compaction input SST");
    }

    assert_eq!(num_files_at_level_cf(&db, COMPACT_CF, 0), 3);
    assert_eq!(collector_ids_from_named_cf(&db, COMPACT_CF).len(), 3);
    assert_eq!(num_active_memtable_entries_cf(&db, COMPACT_CF), 0);
    assert_eq!(num_files_at_level_cf(&db, FLUSH_CF, 0), 0);
    assert!(collector_ids_from_named_cf(&db, FLUSH_CF).is_empty());

    {
        let flush_cf = db
            .cf_handle(FLUSH_CF)
            .expect("get flush column family handle");
        for key_index in 0..128 {
            let key = format!("flush-key-{key_index:04}");
            db.put_cf(&flush_cf, key.as_bytes(), b"pending-manual-flush")
                .expect("write pending flush memtable");
        }
    }
    assert_eq!(num_active_memtable_entries_cf(&db, FLUSH_CF), 128);

    events
        .flushes
        .lock()
        .expect("flush event lock poisoned")
        .clear();
    events
        .compactions
        .lock()
        .expect("compaction event lock poisoned")
        .clear();
    gate.arm();

    let (completion_tx, completion_rx) = mpsc::channel();
    let compact_db = Arc::clone(&db);
    let compact_tx = completion_tx.clone();
    let compact_thread = thread::spawn(move || {
        let compact_cf = compact_db
            .cf_handle(COMPACT_CF)
            .expect("get compact column family handle in worker");
        let mut compact_options = CompactOptions::default();
        compact_options.set_target_level(1);
        compact_options.set_change_level(true);
        compact_options.set_exclusive_manual_compaction(false);
        compact_options.set_bottommost_level_compaction(BottommostLevelCompaction::Skip);
        compact_db.compact_range_cf_opt::<&[u8], &[u8]>(&compact_cf, None, None, &compact_options);
        compact_tx
            .send(("manual compaction", Ok(())))
            .expect("report manual compaction completion");
    });

    let flush_db = Arc::clone(&db);
    let flush_tx = completion_tx;
    let flush_thread = thread::spawn(move || {
        let flush_cf = flush_db
            .cf_handle(FLUSH_CF)
            .expect("get flush column family handle in worker");
        let mut flush_options = FlushOptions::default();
        flush_options.set_wait(true);
        let result = flush_db.flush_cf_opt(&flush_cf, &flush_options);
        flush_tx
            .send(("manual flush", result.map_err(|error| error.to_string())))
            .expect("report manual flush completion");
    });

    let operation_deadline = Instant::now() + OPERATION_TIMEOUT;
    let mut completions = Vec::new();
    while completions.len() < 2 {
        let remaining = operation_deadline.saturating_duration_since(Instant::now());
        match completion_rx.recv_timeout(remaining) {
            Ok(completion) => completions.push(completion),
            Err(mpsc::RecvTimeoutError::Timeout) => break,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let timed_out_before_release = completions.len() != 2;
    let timeout_snapshot = timed_out_before_release.then(|| gate.snapshot());
    if timed_out_before_release {
        gate.force_release();
        let release_deadline = Instant::now() + OPERATION_TIMEOUT;
        while completions.len() < 2 {
            let remaining = release_deadline.saturating_duration_since(Instant::now());
            match completion_rx.recv_timeout(remaining) {
                Ok(completion) => completions.push(completion),
                Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {
                    break;
                }
            }
        }
    }

    if completions.len() == 2 {
        compact_thread
            .join()
            .expect("manual compaction worker must not panic");
        flush_thread
            .join()
            .expect("manual flush worker must not panic");
    }
    assert_eq!(
        completions.len(),
        2,
        "operations did not finish after forcing gate release; before_release={timeout_snapshot:?}; after_release={:?}",
        gate.snapshot()
    );
    assert!(
        !timed_out_before_release,
        "operations exceeded their deadline; before_release={timeout_snapshot:?}; after_release={:?}",
        gate.snapshot()
    );
    for (operation, result) in completions {
        result.unwrap_or_else(|error| panic!("{operation} failed: {error}"));
    }

    let gate_snapshot = gate.snapshot();
    assert!(
        gate_snapshot.max_in_flight >= 2,
        "flush and compaction collector creation did not overlap: {gate_snapshot:?}"
    );
    assert!(!gate_snapshot.timed_out, "{gate_snapshot:?}");
    assert_eq!(gate_snapshot.in_flight, 0, "{gate_snapshot:?}");

    let flush_create = gate_snapshot
        .flush
        .expect("record flush collector creation");
    let compaction_create = gate_snapshot
        .compaction
        .expect("record compaction collector creation");
    assert_eq!(flush_create.context.level_at_creation, 0);
    assert_eq!(compaction_create.context.level_at_creation, 1);
    assert_ne!(
        flush_create.context.column_family_id,
        compaction_create.context.column_family_id
    );
    assert_ne!(flush_create.collector_id, compaction_create.collector_id);

    assert_eq!(num_files_at_level_cf(&db, COMPACT_CF, 0), 0);
    assert!(num_files_at_level_cf(&db, COMPACT_CF, 1) >= 1);
    assert_eq!(num_files_at_level_cf(&db, FLUSH_CF, 0), 1);
    assert_eq!(
        collector_ids_from_named_cf(&db, COMPACT_CF),
        vec![compaction_create.collector_id]
    );
    assert_eq!(
        collector_ids_from_named_cf(&db, FLUSH_CF),
        vec![flush_create.collector_id]
    );

    let flush_events = events
        .flushes
        .lock()
        .expect("flush event lock poisoned")
        .clone();
    assert!(flush_events.iter().any(|event| {
        event.cf_name.as_deref() == Some(FLUSH_CF.as_bytes())
            && event.reason == DBFlushReason::KManualFlush
    }));
    let compaction_events = events
        .compactions
        .lock()
        .expect("compaction event lock poisoned")
        .clone();
    assert!(compaction_events.iter().any(|event| {
        event.cf_name.as_deref() == Some(COMPACT_CF.as_bytes())
            && event.reason == DBCompactionReason::KManualCompaction
            && event.status_ok
            && event.base_input_level == 0
            && event.output_level == 1
            && event.input_file_count >= 2
            && event.output_file_count >= 1
    }));

    assert_eq!(
        counts.collectors_created.load(Ordering::SeqCst),
        counts.collectors_dropped.load(Ordering::SeqCst)
    );
    assert_eq!(counts.factories_dropped.load(Ordering::SeqCst), 0);
    drop(db);
    assert_eq!(counts.factories_dropped.load(Ordering::SeqCst), 1);
}

#[test]
fn factory_create_overlap_subprocess_entry() {
    if env::var_os(OVERLAP_CHILD_ENV).is_none() {
        return;
    }
    let db_path =
        env::var_os(OVERLAP_CHILD_DB_ENV).expect("overlap child database path must be provided");
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{OVERLAP_CHILD_STARTED_MARKER}").expect("write overlap child start marker");
    stdout.flush().expect("flush overlap child start marker");
    drop(stdout);

    run_factory_create_overlap_scenario(Path::new(&db_path));

    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{OVERLAP_CHILD_SUCCEEDED_MARKER}")
        .expect("write overlap child success marker");
    stdout.flush().expect("flush overlap child success marker");
}

#[test]
fn factory_create_overlaps_between_real_flush_and_manual_compaction() {
    if skip_bundled_backend_only_test() {
        return;
    }

    let path = DBPath::new("_rust_rocksdb_collector_flush_compaction_overlap");
    let timeout = Duration::from_secs(75);
    let path_ref = &path;
    let db_path: &Path = path_ref.as_ref();
    let child_reaper =
        child_reaper_sender().expect("start child reaper before spawning overlap child process");
    let child = Command::new(env::current_exe().expect("locate current test executable"))
        .args([
            "--exact",
            "factory_create_overlap_subprocess_entry",
            "--nocapture",
        ])
        .env(OVERLAP_CHILD_ENV, "1")
        .env(OVERLAP_CHILD_DB_ENV, db_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn collector overlap child process");
    let watchdog = wait_for_child_with_watchdog(child, timeout, Child::try_wait, child_reaper)
        .expect("wait for collector overlap child process");
    let output = &watchdog.output;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !watchdog.timed_out,
        "collector overlap child exceeded {timeout:?}; stdout={stdout}; stderr={stderr}"
    );
    assert!(
        watchdog.poll_error.is_none(),
        "poll collector overlap child process: {:?}; stdout={stdout}; stderr={stderr}",
        watchdog.poll_error
    );
    assert!(
        output.status.success(),
        "collector overlap child failed with {}; stdout={stdout}; stderr={stderr}",
        output.status
    );
    assert!(
        stdout.contains(OVERLAP_CHILD_STARTED_MARKER),
        "collector overlap child did not emit its start marker; stdout={stdout}; stderr={stderr}"
    );
    assert!(
        stdout.contains(OVERLAP_CHILD_SUCCEEDED_MARKER),
        "collector overlap child did not emit its success marker; stdout={stdout}; stderr={stderr}"
    );
}

#[test]
fn drop_options_clone_and_multi_cf_db_release_factory_and_collectors_once() {
    if skip_bundled_backend_only_test() {
        return;
    }

    let path = DBPath::new("_rust_rocksdb_collector_drop_normal");
    let counts = Arc::new(LifecycleCounts::default());
    let mut original_options = Options::default();
    original_options.create_if_missing(true);
    original_options.create_missing_column_families(true);
    original_options.set_max_background_jobs(4);
    original_options.set_table_properties_collector_factory(CountingFactory {
        counts: Arc::clone(&counts),
    });
    let cloned_options = original_options.clone();
    drop(original_options);

    assert_eq!(counts.factories_dropped.load(Ordering::SeqCst), 0);

    let descriptors = ["default", "cf1", "cf2"]
        .map(|name| ColumnFamilyDescriptor::new(name, cloned_options.clone()));
    let db = DB::open_cf_descriptors(&cloned_options, &path, descriptors)
        .expect("open lifecycle test database");
    drop(cloned_options);

    db.put(b"default-key", b"default-value")
        .expect("put default value");
    {
        let cf1 = db.cf_handle("cf1").expect("get cf1 handle");
        let cf2 = db.cf_handle("cf2").expect("get cf2 handle");
        db.put_cf(&cf1, b"cf1-key", b"cf1-value")
            .expect("put cf1 value");
        db.put_cf(&cf2, b"cf2-key", b"cf2-value")
            .expect("put cf2 value");
        let default_cf = db.cf_handle("default").expect("get default cf handle");
        let mut flush_options = FlushOptions::default();
        flush_options.set_wait(true);
        db.flush_cfs_opt(&[&default_cf, &cf1, &cf2], &flush_options)
            .expect("flush lifecycle test column families");
    }

    assert_eq!(counts.collectors_created.load(Ordering::SeqCst), 3);
    assert_eq!(counts.collectors_dropped.load(Ordering::SeqCst), 3);
    assert_eq!(counts.factories_dropped.load(Ordering::SeqCst), 0);

    drop(db);

    assert_eq!(counts.factories_dropped.load(Ordering::SeqCst), 1);
    assert_eq!(counts.collectors_created.load(Ordering::SeqCst), 3);
    assert_eq!(counts.collectors_dropped.load(Ordering::SeqCst), 3);
}

#[test]
fn drop_failed_open_releases_factory_once_without_creating_collector() {
    if skip_bundled_backend_only_test() {
        return;
    }

    let invalid_db_path = tempfile::NamedTempFile::new().expect("create invalid DB file path");
    let counts = Arc::new(LifecycleCounts::default());
    let mut options = Options::default();
    options.create_if_missing(true);
    options.set_table_properties_collector_factory(CountingFactory {
        counts: Arc::clone(&counts),
    });

    let error = DB::open(&options, invalid_db_path.path())
        .expect_err("opening a database on an existing regular file must fail");
    assert!(!error.to_string().is_empty());
    assert_eq!(counts.factories_dropped.load(Ordering::SeqCst), 0);
    assert_eq!(counts.collectors_created.load(Ordering::SeqCst), 0);
    assert_eq!(counts.collectors_dropped.load(Ordering::SeqCst), 0);

    drop(options);

    assert_eq!(counts.factories_dropped.load(Ordering::SeqCst), 1);
    assert_eq!(counts.collectors_created.load(Ordering::SeqCst), 0);
    assert_eq!(counts.collectors_dropped.load(Ordering::SeqCst), 0);
}

#[test]
fn closure_callback_preserves_the_original_public_adapter() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let observed_for_add = Arc::clone(&observed);
    let mut collector = TablePropertiesCollectorCallback {
        name: CString::new("callback").expect("callback name must not contain NUL"),
        add_fn: move |key: &[u8], _value: &[u8], entry_type, sequence, file_size| {
            observed_for_add.lock().expect("probe lock poisoned").push((
                key.to_vec(),
                entry_type,
                sequence,
                file_size,
            ));
        },
        finish_fn: || HashMap::from([(b"binary".to_vec(), b"value".to_vec())]),
        get_readable_fn: || HashMap::from([(b"readable".to_vec(), b"value".to_vec())]),
    };

    collector.add(b"key", b"value", DBEntryType::Put, 11, 12);

    assert_eq!(collector.name(), c"callback");
    assert_eq!(collector.finish()[b"binary".as_slice()], b"value");
    assert_eq!(
        collector.get_readable_properties()[b"readable".as_slice()],
        b"value"
    );
    assert_eq!(
        *observed.lock().expect("probe lock poisoned"),
        vec![(b"key".to_vec(), DBEntryType::Put, 11, 12)]
    );
}
