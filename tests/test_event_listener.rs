mod util;

use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::*;

use rust_rocksdb::{DB, FlushOptions, Options, event_listener::*};

use util::DBPath;

const PANIC_CHILD_ENV: &str = "RUST_ROCKSDB_EVENT_LISTENER_PANIC_CHILD";
const PANIC_CHILD_DB_ENV: &str = "RUST_ROCKSDB_EVENT_LISTENER_PANIC_DB";
const PANIC_CHILD_MARKER: &str = "rust-rocksdb event-listener panic child entered";
const FLUSH_BEGIN_PANIC_DIAGNOSTIC: &str =
    "rust-rocksdb: event listener on_flush_begin callback panicked";
const DESTRUCTOR_PANIC_DIAGNOSTIC: &str =
    "rust-rocksdb: event listener destructor callback panicked";

#[derive(Default, Clone)]
struct EventCounter {
    flush: Arc<AtomicUsize>,
    compaction: Arc<AtomicUsize>,
    ingestion: Arc<AtomicUsize>,
    input_records: Arc<AtomicUsize>,
    output_records: Arc<AtomicUsize>,
    input_bytes: Arc<AtomicUsize>,
    output_bytes: Arc<AtomicUsize>,
    manual_compaction: Arc<AtomicUsize>,
    manual_flush: Arc<AtomicUsize>,
}

impl EventListener for EventCounter {
    fn on_memtable_sealed(&self, info: &MemTableInfo) {
        assert!(!info.cf_name().unwrap().is_empty());
    }

    fn on_flush_begin(&self, info: &FlushJobInfo) {
        assert!(!info.cf_name().unwrap().is_empty());
        // https://github.com/facebook/rocksdb/issues/11568#issuecomment-1614995815
        assert_eq!(info.smallest_seqno(), 72057594037927935); // default value
        assert_eq!(info.largest_seqno(), 0); // default value
    }

    fn on_flush_completed(&self, info: &FlushJobInfo) {
        assert!(!info.cf_name().unwrap().is_empty());
        self.flush.fetch_add(1, Ordering::SeqCst);
        assert_ne!(info.largest_seqno(), 0);
        assert!(info.smallest_seqno() <= info.largest_seqno());

        if info.flush_reason() == DBFlushReason::KManualFlush {
            self.manual_flush.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn on_compaction_completed(&self, info: &CompactionJobInfo) {
        info.status().unwrap();
        assert!(!info.cf_name().unwrap().is_empty());
        let input_file_count = info.input_file_count();
        assert_ne!(input_file_count, 0);
        assert_eq!(info.num_input_files_at_output_level(), 0);

        let output_file_count = info.output_file_count();
        assert_ne!(output_file_count, 0);

        assert_ne!(info.elapsed_micros(), 0);
        assert_eq!(info.num_corrupt_keys(), 0);
        assert!(info.output_level() >= 0);

        self.compaction.fetch_add(1, Ordering::SeqCst);
        self.input_records
            .fetch_add(info.input_records() as usize, Ordering::SeqCst);
        self.output_records
            .fetch_add(info.output_records() as usize, Ordering::SeqCst);
        self.input_bytes
            .fetch_add(info.total_input_bytes() as usize, Ordering::SeqCst);
        self.output_bytes
            .fetch_add(info.total_output_bytes() as usize, Ordering::SeqCst);

        if info.compaction_reason() == DBCompactionReason::KManualCompaction {
            self.manual_compaction.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn on_external_file_ingested(&self, info: &IngestionInfo) {
        assert!(!info.cf_name().unwrap().is_empty());
        self.ingestion.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default, Clone)]
struct StallEventCounter {
    flush: Arc<AtomicUsize>,
    stall_conditions_changed_num: Arc<AtomicUsize>,
    triggered_writes_slowdown: Arc<AtomicUsize>,
    triggered_writes_stop: Arc<AtomicUsize>,
    stall_change_from_normal_to_other: Arc<AtomicUsize>,
}

impl EventListener for StallEventCounter {
    fn on_flush_completed(&self, info: &FlushJobInfo) {
        assert!(!info.cf_name().unwrap().is_empty());
        self.flush.fetch_add(1, Ordering::SeqCst);
        self.triggered_writes_slowdown
            .fetch_add(info.triggered_writes_slowdown() as usize, Ordering::SeqCst);
        self.triggered_writes_stop
            .fetch_add(info.triggered_writes_stop() as usize, Ordering::SeqCst);
    }

    fn on_stall_conditions_changed(&self, info: &WriteStallInfo) {
        assert!(info.cf_name() == Some("test_cf".as_bytes().to_vec()));
        self.stall_conditions_changed_num
            .fetch_add(1, Ordering::SeqCst);
        if info.prev() == DBWriteStallCondition::KNormal
            && info.cur() != DBWriteStallCondition::KNormal
        {
            self.stall_change_from_normal_to_other
                .fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[derive(Default, Clone)]
struct BackgroundErrorCounter {
    background_error: Arc<AtomicUsize>,
}

impl EventListener for BackgroundErrorCounter {
    fn on_background_error(&self, _: DBBackgroundErrorReason, _: &MutableStatus) {
        self.background_error.fetch_add(1, Ordering::SeqCst);
    }
}

struct OwnershipListener {
    callbacks: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

impl EventListener for OwnershipListener {
    fn on_flush_completed(&self, _: &FlushJobInfo) {
        self.callbacks.fetch_add(1, Ordering::SeqCst);
    }
}

impl Drop for OwnershipListener {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

struct FlushBeginPanickingListener;

impl EventListener for FlushBeginPanickingListener {
    fn on_flush_begin(&self, _: &FlushJobInfo) {
        panic!("flush begin callback panic");
    }
}

struct DestructorPanickingListener;

impl EventListener for DestructorPanickingListener {}

impl Drop for DestructorPanickingListener {
    fn drop(&mut self) {
        panic!("listener destructor panic");
    }
}

fn run_panic_child(mode: &str, db_path: &Path) -> std::process::Output {
    Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("event_listener_panic_child")
        .arg("--nocapture")
        .env(PANIC_CHILD_ENV, mode)
        .env(PANIC_CHILD_DB_ENV, db_path)
        .output()
        .unwrap()
}

#[test]
fn test_event_listener_flush_begin_panic_aborts_with_diagnostic() {
    let path = DBPath::new("_rust_rocksdb_event_listener_flush_begin_panic");
    let output = run_panic_child("flush_begin", (&path).as_ref());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "child unexpectedly succeeded; stdout={stdout}; stderr={stderr}"
    );
    assert!(
        stdout.contains(PANIC_CHILD_MARKER),
        "child stdout did not confirm entry into the panic scenario; stdout={stdout}; stderr={stderr}"
    );
    assert!(
        stderr.contains(FLUSH_BEGIN_PANIC_DIAGNOSTIC),
        "child stderr did not contain the fail-fast diagnostic; stdout={stdout}; stderr={stderr}"
    );
}

#[test]
fn test_event_listener_destructor_panic_aborts_with_diagnostic() {
    let path = DBPath::new("_rust_rocksdb_event_listener_destructor_panic");
    let output = run_panic_child("destructor", (&path).as_ref());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "child unexpectedly succeeded; stdout={stdout}; stderr={stderr}"
    );
    assert!(
        stdout.contains(PANIC_CHILD_MARKER),
        "child stdout did not confirm entry into the panic scenario; stdout={stdout}; stderr={stderr}"
    );
    assert!(
        stderr.contains(DESTRUCTOR_PANIC_DIAGNOSTIC),
        "child stderr did not contain the fail-fast diagnostic; stdout={stdout}; stderr={stderr}"
    );
}

#[test]
fn event_listener_panic_child() {
    let mode = match std::env::var(PANIC_CHILD_ENV) {
        Ok(mode) if mode == "flush_begin" || mode == "destructor" => mode,
        _ => return,
    };
    let db_path = std::env::var_os(PANIC_CHILD_DB_ENV)
        .expect("event listener panic child database path must be provided");
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{PANIC_CHILD_MARKER}").expect("write event listener panic child marker");
    stdout
        .flush()
        .expect("flush event listener panic child marker");
    drop(stdout);

    match mode.as_str() {
        "flush_begin" => {
            let mut opts = Options::default();
            opts.create_if_missing(true);
            opts.add_event_listener(FlushBeginPanickingListener);
            let db = DB::open(&opts, &db_path).unwrap();
            db.put(b"key", b"value").unwrap();
            let mut flush_opts = FlushOptions::default();
            flush_opts.set_wait(true);
            db.flush_opt(&flush_opts).unwrap();
            panic!("flush begin callback did not abort the process");
        }
        "destructor" => {
            let mut opts = Options::default();
            opts.create_if_missing(true);
            opts.add_event_listener(DestructorPanickingListener);
            let db = DB::open(&opts, &db_path).unwrap();
            drop(opts);
            drop(db);
            panic!("listener destructor did not abort the process");
        }
        _ => unreachable!(),
    }
}

#[test]
fn test_event_listener_stall_conditions_changed() {
    let path = DBPath::new("_rust_rocksdb_event_listener_stall_conditions");

    let mut opts = Options::default();
    let counter = StallEventCounter::default();
    opts.add_event_listener(counter.clone());
    opts.create_if_missing(true);
    let mut cf_opts = Options::default();
    cf_opts.set_level_zero_slowdown_writes_trigger(1);
    cf_opts.set_level_zero_stop_writes_trigger(1);
    cf_opts.set_level_zero_file_num_compaction_trigger(1);

    #[cfg(feature = "multi-threaded-cf")]
    let db = DB::open(&opts, &path).unwrap();
    #[cfg(not(feature = "multi-threaded-cf"))]
    let mut db = DB::open(&opts, &path).unwrap();
    db.create_cf("test_cf", &cf_opts).unwrap();

    let test_cf = db.cf_handle("test_cf").unwrap();
    for i in 1..5 {
        db.put_cf(
            &test_cf,
            format!("{i:04}").as_bytes(),
            format!("{i:04}").as_bytes(),
        )
        .unwrap();
        let mut fopts = FlushOptions::default();
        fopts.set_wait(true);
        db.flush_cf_opt(&test_cf, &fopts).unwrap();
    }

    let flush_cnt = counter.flush.load(Ordering::SeqCst);
    assert_ne!(flush_cnt, 0);
    let stall_conditions_changed_num = counter.stall_conditions_changed_num.load(Ordering::SeqCst);
    let triggered_writes_slowdown = counter.triggered_writes_slowdown.load(Ordering::SeqCst);
    let triggered_writes_stop = counter.triggered_writes_stop.load(Ordering::SeqCst);
    let stall_change_from_normal_to_other = counter
        .stall_change_from_normal_to_other
        .load(Ordering::SeqCst);
    assert_ne!(stall_conditions_changed_num, 0);
    assert_ne!(triggered_writes_slowdown, 0);
    assert_ne!(triggered_writes_stop, 0);
    assert_ne!(stall_change_from_normal_to_other, 0);
}

#[test]
fn test_event_listener_basic() {
    let path = DBPath::new("_rust_rocksdb_event_listener_flush");

    let mut opts = Options::default();
    let counter = EventCounter::default();
    opts.add_event_listener(counter.clone());
    opts.create_if_missing(true);
    let db = DB::open(&opts, &path).unwrap();
    for i in 1..8000 {
        db.put(format!("{i:04}").as_bytes(), format!("{i:04}").as_bytes())
            .unwrap();
    }
    let mut fopts = FlushOptions::default();
    fopts.set_wait(true);
    db.flush_opt(&fopts).unwrap();
    assert_ne!(counter.flush.load(Ordering::SeqCst), 0);
    assert_eq!(counter.manual_flush.load(Ordering::SeqCst), 1);

    for i in 1..8000 {
        db.put(format!("{i:04}").as_bytes(), format!("{i:04}").as_bytes())
            .unwrap();
    }
    db.flush_opt(&fopts).unwrap();
    let flush_cnt = counter.flush.load(Ordering::SeqCst);
    assert_ne!(flush_cnt, 0);
    assert_eq!(counter.compaction.load(Ordering::SeqCst), 0);
    db.compact_range(None::<&[u8]>, None::<&[u8]>);
    assert_eq!(counter.flush.load(Ordering::SeqCst), flush_cnt);
    assert_eq!(counter.manual_flush.load(Ordering::SeqCst), 2);
    assert_ne!(counter.compaction.load(Ordering::SeqCst), 0);
    drop(db);
    assert!(
        counter.input_records.load(Ordering::SeqCst)
            > counter.output_records.load(Ordering::SeqCst)
    );
    assert!(
        counter.input_bytes.load(Ordering::SeqCst) > counter.output_bytes.load(Ordering::SeqCst)
    );
    assert_eq!(counter.manual_compaction.load(Ordering::SeqCst), 1);
}

#[test]
fn test_event_listener_ownership_survives_options_drop() {
    let path = DBPath::new("_rust_rocksdb_event_listener_ownership");
    let callbacks = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));

    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.add_event_listener(OwnershipListener {
        callbacks: callbacks.clone(),
        drops: drops.clone(),
    });

    let db = DB::open(&opts, &path).unwrap();
    drop(opts);
    assert_eq!(drops.load(Ordering::SeqCst), 0);

    db.put(b"key", b"value").unwrap();
    let mut flush_opts = FlushOptions::default();
    flush_opts.set_wait(true);
    db.flush_opt(&flush_opts).unwrap();
    assert_ne!(callbacks.load(Ordering::SeqCst), 0);

    drop(db);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn test_event_listener_background_error() {
    let path = DBPath::new("_rust_rocksdb_event_listener_background_error");

    let mut opts = Options::default();
    let counter = BackgroundErrorCounter::default();
    opts.add_event_listener(counter.clone());
    opts.create_if_missing(true);
    let db = DB::open(&opts, &path).unwrap();

    let mut fopts = FlushOptions::default();
    fopts.set_wait(true);
    for i in 1..10 {
        db.put(format!("{i:04}").as_bytes(), b"value").unwrap();
        db.flush_opt(&fopts).unwrap();
    }
    assert_eq!(counter.background_error.load(Ordering::SeqCst), 0);
}

#[test]
fn test_background_error_reason_async_file_open() {
    assert_eq!(
        DBBackgroundErrorReason::from(7),
        DBBackgroundErrorReason::KAsyncFileOpen
    );
}

#[derive(Clone)]
struct BackgroundErrorCleaner {
    count: Arc<AtomicUsize>,
    severity: Arc<AtomicUsize>,
    recovery_begin: Arc<AtomicUsize>,
    recovery_end: Arc<AtomicUsize>,
}

impl Default for BackgroundErrorCleaner {
    fn default() -> Self {
        Self {
            count: Arc::new(AtomicUsize::new(0)),
            severity: Arc::new(AtomicUsize::new(StatusSeverity::KUnknown as usize)),
            recovery_begin: Arc::new(AtomicUsize::new(0)),
            recovery_end: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl EventListener for BackgroundErrorCleaner {
    fn on_background_error(&self, _: DBBackgroundErrorReason, s: &MutableStatus) {
        assert!(s.result().is_err());
        self.severity.store(s.severity() as usize, Ordering::SeqCst);
        s.reset();
        assert_eq!(s.severity(), StatusSeverity::KNoError);
        self.count.fetch_add(1, Ordering::SeqCst);
    }

    fn on_error_recovery_begin(
        &self,
        _: DBBackgroundErrorReason,
        status: &BackgroundErrorStatus,
        auto_recovery: &mut bool,
    ) {
        assert!(status.result().is_err());
        assert_ne!(status.severity(), StatusSeverity::KNoError);
        *auto_recovery = true;
        self.recovery_begin.fetch_add(1, Ordering::SeqCst);
    }

    fn on_error_recovery_end(&self, info: &BackgroundErrorRecoveryInfo) {
        assert!(info.old_bg_error().result().is_err());
        assert_ne!(info.old_bg_error().severity(), StatusSeverity::KNoError);
        self.recovery_end.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn test_event_listener_status_reset() {
    let path = DBPath::new("_rust_rocksdb_event_listener_background_error");

    let mut opts = Options::default();
    let cleaner = BackgroundErrorCleaner::default();
    let counter = cleaner.count.clone();
    let severity = cleaner.severity.clone();
    opts.add_event_listener(cleaner.clone());
    opts.create_if_missing(true);
    let db = DB::open(&opts, &path).unwrap();

    for i in 1..5 {
        db.put(format!("{i:04}").as_bytes(), b"value").unwrap();
    }
    let mut fopts = FlushOptions::default();
    fopts.set_wait(true);
    db.flush_opt(&fopts).unwrap();

    corrupt_sst_file(&db, &path);

    for i in 1..5 {
        db.put(format!("{i:04}").as_bytes(), b"value").unwrap();
    }

    db.compact_range(None::<&[u8]>, None::<&[u8]>);

    db.flush_opt(&fopts).unwrap();
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    assert!(
        StatusSeverity::from(severity.load(Ordering::SeqCst) as u8) >= StatusSeverity::KHardError
    );
}

fn corrupt_sst_file<P: AsRef<Path>>(db: &DB, path: P) {
    let files = db.live_files().unwrap();
    let mut file_name = files.first().unwrap().name.clone();
    file_name.remove(0);

    let sst_path = path.as_ref().to_path_buf().join(file_name);

    let mut file = std::fs::File::create(sst_path).unwrap();
    file.write_all(b"sad").unwrap();
    file.sync_all().unwrap();
}
