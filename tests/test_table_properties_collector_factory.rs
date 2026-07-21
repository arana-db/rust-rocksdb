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
    io::{self, Write},
    path::Path,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

use rust_rocksdb::{
    BlockBasedOptions, ColumnFamilyDescriptor, DB, FlushOptions, MergeOperands, Options,
    WriteBatch,
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
const CHILD_STARTED_MARKER: &str = "COLLECTOR_CHILD_STARTED";
const FLUSH_SUCCEEDED_MARKER: &str = "FLUSH_SUCCEEDED";

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

    assert_all_installed_ssts_have_kiwi_property(db_path);
}

fn assert_collector_contract<T: TablePropertiesCollector + Send + 'static>() {}
fn assert_factory_contract<T: TablePropertiesCollectorFactory + Send + Sync + 'static>() {}

#[test]
fn collector_subprocess_entry() {
    let Ok(case) = env::var(COLLECTOR_PANIC_CASE_ENV) else {
        return;
    };
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
fn bundled_backend_reports_support() {
    let supported =
        unsafe { rust_librocksdb_sys::rust_rocksdb_table_properties_collector_factory_supported() };

    assert_eq!(supported, 1);
}

#[test]
fn collector_factory_writes_kiwi_property_bytes_during_flush() {
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

#[test]
fn drop_options_clone_and_multi_cf_db_release_factory_and_collectors_once() {
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
