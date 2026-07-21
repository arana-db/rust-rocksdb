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
    ffi::{CStr, CString},
    sync::{Arc, Mutex},
};

use rust_rocksdb::{
    DB, Options,
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

fn assert_collector_contract<T: TablePropertiesCollector + Send + 'static>() {}
fn assert_factory_contract<T: TablePropertiesCollectorFactory + Send + Sync + 'static>() {}

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
