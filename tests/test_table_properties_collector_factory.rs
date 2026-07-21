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
    table_properties_collector::{
        DBEntryType, TablePropertiesCollector, TablePropertiesCollectorCallback,
    },
    table_properties_collector_factory::{
        TablePropertiesCollectorContext, TablePropertiesCollectorFactory,
    },
};

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
