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

mod util;

use std::collections::HashMap;

use rust_rocksdb::{
    BlockBasedOptions, DB, MergeOperands, Options, TableProperties, TablePropertiesCollection,
    WriteBatch,
};
use util::DBPath;

fn open_db(name: &str) -> (DBPath, DB) {
    let path = DBPath::new(name);
    let mut options = Options::default();
    options.create_if_missing(true);
    let db = DB::open(&options, &path).expect("open test database");
    (path, db)
}

fn assert_numeric_properties(properties: &TableProperties) {
    assert!(properties.data_size() > 0);
    assert!(properties.index_size() > 0);
    assert!(properties.raw_key_size() > 0);
    assert!(properties.raw_value_size() > 0);
    assert!(properties.num_data_blocks() > 0);
    assert!(properties.num_entries() > 0);
    assert_eq!(properties.num_deletions(), 0);
    assert_eq!(properties.num_merge_operands(), 0);
    assert_eq!(properties.num_range_deletions(), 0);
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

#[test]
fn empty_database_returns_empty_collection() {
    let (_path, db) = open_db("_rust_rocksdb_table_properties_empty");
    let collection: TablePropertiesCollection = db
        .get_properties_of_all_tables()
        .expect("read table properties from empty database");

    assert_eq!(collection.len(), 0);
    assert!(collection.is_empty());
    assert_eq!(collection.iter().count(), 0);
}

#[test]
fn reads_default_column_family_properties() {
    let (_path, db) = open_db("_rust_rocksdb_table_properties_default_cf");
    db.put(b"key-1", b"value-1").expect("put key-1");
    db.put(b"key-2", b"value-2").expect("put key-2");
    db.flush().expect("flush default column family");

    let collection = db
        .get_properties_of_all_tables()
        .expect("read default column family table properties");

    assert!(!collection.is_empty());
    assert_eq!(collection.len(), collection.iter().count());

    let mut total_entries = 0;
    for (file_name, properties) in collection.iter() {
        let file_name: Box<[u8]> = file_name;
        assert!(!file_name.is_empty());
        assert_numeric_properties(&properties);
        total_entries += properties.num_entries();

        let user: HashMap<Vec<u8>, Vec<u8>> = properties.user_collected_properties();
        let readable: HashMap<Vec<u8>, Vec<u8>> = properties.readable_properties();
        assert_eq!(
            user.get(b"rocksdb.block.based.table.index.type".as_slice()),
            Some(&vec![0, 0, 0, 0])
        );
        assert_eq!(
            user.get(b"rocksdb.block.based.table.whole.key.filtering".as_slice()),
            Some(&b"1".to_vec())
        );
        assert_eq!(
            user.get(b"rocksdb.block.based.table.prefix.filtering".as_slice()),
            Some(&b"0".to_vec())
        );
        assert!(readable.is_empty());
    }
    assert_eq!(total_entries, 2);
}

#[test]
fn reads_only_the_named_column_family_properties() {
    let path = DBPath::new("_rust_rocksdb_table_properties_named_cf");
    let mut options = Options::default();
    options.create_if_missing(true);
    options.create_missing_column_families(true);

    let db = DB::open_cf(&options, &path, ["cf1"]).expect("open database with cf1");
    db.put(b"default-key", b"default-value")
        .expect("put default value");
    db.flush().expect("flush default column family");

    let cf = db.cf_handle("cf1").expect("get cf1 handle");
    for index in 0..3 {
        db.put_cf(
            &cf,
            format!("cf-key-{index}").as_bytes(),
            format!("cf-value-{index}").as_bytes(),
        )
        .expect("put cf1 value");
    }
    db.flush_cf(&cf).expect("flush cf1");

    let collection = db
        .get_properties_of_all_tables_cf(&cf)
        .expect("read cf1 table properties");
    let cf_entries: u64 = collection
        .iter()
        .map(|(_, properties)| properties.num_entries())
        .sum();
    let default_entries: u64 = db
        .get_properties_of_all_tables()
        .expect("read default table properties")
        .iter()
        .map(|(_, properties)| properties.num_entries())
        .sum();

    assert_eq!(collection.len(), 1);
    assert_eq!(cf_entries, 3);
    assert_eq!(default_entries, 1);
}

#[test]
fn maps_delete_merge_and_range_delete_counts_to_the_correct_getters() {
    let path = DBPath::new("_rust_rocksdb_table_properties_entry_kinds");
    let mut options = Options::default();
    options.create_if_missing(true);
    options.set_merge_operator_associative("concat", concat_merge);
    let db = DB::open(&options, &path).expect("open test database");

    db.put(b"delete-key", b"value").expect("put delete key");
    db.put(b"range-a", b"value").expect("put range-a");
    db.put(b"range-c", b"value").expect("put range-c");
    db.put(b"range-e", b"value").expect("put range-e");
    db.flush().expect("flush initial values");

    db.delete(b"delete-key").expect("delete key");
    db.merge(b"merge-key-1", b"operand-1")
        .expect("merge operand 1");
    db.merge(b"merge-key-2", b"operand-2")
        .expect("merge operand 2");
    let mut batch = WriteBatch::default();
    batch.delete_range(b"range-a", b"range-b");
    batch.delete_range(b"range-c", b"range-d");
    batch.delete_range(b"range-e", b"range-f");
    db.write(&batch).expect("write range deletion");
    db.flush().expect("flush entry kinds");

    let collection = db
        .get_properties_of_all_tables()
        .expect("read entry kind properties");
    let deletions: u64 = collection
        .iter()
        .map(|(_, properties)| properties.num_deletions())
        .sum();
    let merge_operands: u64 = collection
        .iter()
        .map(|(_, properties)| properties.num_merge_operands())
        .sum();
    let range_deletions: u64 = collection
        .iter()
        .map(|(_, properties)| properties.num_range_deletions())
        .sum();

    // RocksDB counts range tombstones as deletions as well as range deletions.
    assert_eq!(deletions, 4);
    assert_eq!(merge_operands, 2);
    assert_eq!(range_deletions, 3);
}

#[test]
fn reads_nonzero_filter_size_from_a_bloom_filtered_table() {
    let path = DBPath::new("_rust_rocksdb_table_properties_filter_size");
    let mut block_options = BlockBasedOptions::default();
    block_options.set_bloom_filter(10.0, false);

    let mut options = Options::default();
    options.create_if_missing(true);
    options.set_block_based_table_factory(&block_options);
    let db = DB::open(&options, &path).expect("open bloom-filtered database");

    for index in 0..100 {
        db.put(
            format!("key-{index:03}").as_bytes(),
            format!("value-{index:03}").as_bytes(),
        )
        .expect("put bloom-filtered value");
    }
    db.flush().expect("flush bloom-filtered table");

    let filter_size: u64 = db
        .get_properties_of_all_tables()
        .expect("read bloom-filtered table properties")
        .iter()
        .map(|(_, properties)| properties.filter_size())
        .sum();

    assert!(filter_size > 0);
}

#[test]
fn borrowed_iterator_can_be_dropped_without_invalidating_collection() {
    let (_path, db) = open_db("_rust_rocksdb_table_properties_borrowed_iter");
    db.put(b"key", b"value").expect("put value");
    db.flush().expect("flush database");

    let collection = db
        .get_properties_of_all_tables()
        .expect("read table properties");

    let mut iterator = collection.iter();
    assert!(iterator.next().is_some());
    drop(iterator);

    assert_eq!(collection.iter().count(), collection.len());
}

#[test]
fn owning_iterator_keeps_collection_alive_until_drop() {
    let (_path, db) = open_db("_rust_rocksdb_table_properties_owning_iter");
    db.put(b"key", b"value").expect("put value");
    db.flush().expect("flush database");

    let collection = db
        .get_properties_of_all_tables()
        .expect("read table properties");
    let expected_len = collection.len();
    let mut iterator = collection.into_iter();

    assert!(iterator.next().is_some());
    let consumed = 1 + iterator.count();
    assert_eq!(consumed, expected_len);
}

#[test]
fn table_properties_item_outlives_consuming_iterator() {
    let (path, db) = open_db("_rust_rocksdb_table_properties_owned_item");
    db.put(b"key", b"value").expect("put value");
    db.flush().expect("flush database");

    let properties = {
        let collection = db
            .get_properties_of_all_tables()
            .expect("read table properties");
        let (_file_name, properties) = collection
            .into_iter()
            .next()
            .expect("read one table property");
        properties
    };
    drop(db);
    drop(path);

    assert_numeric_properties(&properties);
}

#[test]
fn partially_consumed_owning_iterator_can_be_dropped() {
    let (_path, db) = open_db("_rust_rocksdb_table_properties_partial_into_iter");
    db.put(b"key", b"value").expect("put value");
    db.flush().expect("flush database");

    let collection = db
        .get_properties_of_all_tables()
        .expect("read table properties");
    let mut iterator = collection.into_iter();
    let _ = iterator.next();
    drop(iterator);

    let replacement = db
        .get_properties_of_all_tables()
        .expect("read table properties after dropping partial iterator");
    assert!(!replacement.is_empty());
}
