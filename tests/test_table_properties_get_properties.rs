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

use rust_rocksdb::{DB, Options};
use util::DBPath;

/// Test that we can get properties for all tables after flushing data
#[test]
fn test_get_properties_of_all_tables_basic() {
    let path = DBPath::new("_rust_rocksdb_get_properties_basic");

    let mut opts = Options::default();
    opts.create_if_missing(true);

    let db = DB::open(&opts, &path).unwrap();

    // Write some data
    db.put(b"key1", b"value1").unwrap();
    db.put(b"key2", b"value2").unwrap();
    db.put(b"key3", b"value3").unwrap();

    // Flush to create SST files
    db.flush().unwrap();

    // Get properties for all tables
    let collection = db.get_properties_of_all_tables().unwrap();

    // Should have at least one table
    assert!(!collection.is_empty(), "Expected at least one SST file");
    assert!(collection.len() >= 1, "Expected at least one table");

    // Iterate and check properties
    let mut found_entries = false;
    for (table_name, props) in collection.iter() {
        // Table name should be a valid string
        assert!(!table_name.is_empty(), "Table name should not be empty");

        // Check that we have some entries
        if props.num_entries() > 0 {
            found_entries = true;
        }

        // Verify numeric properties are reasonable
        assert!(props.data_size() > 0, "Data size should be > 0");
    }

    assert!(found_entries, "Expected to find at least one entry");
}

/// Test that we can get properties with column family
#[test]
fn test_get_properties_of_all_tables_cf() {
    let path = DBPath::new("_rust_rocksdb_get_properties_cf");

    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);

    // Open database with a custom column family
    let db = DB::open_cf(&opts, &path, ["cf1"]).unwrap();

    // Get column family handle
    let cf = db.cf_handle("cf1").unwrap();

    // Write data to the column family and flush
    db.put_cf(&cf, b"key1", b"value1").unwrap();
    db.flush_cf(&cf).unwrap();

    // Get properties for the column family
    let collection = db.get_properties_of_all_tables_cf(&cf).unwrap();

    // Should have tables
    assert!(!collection.is_empty());
}

/// Test that collection can be iterated with IntoIterator
#[test]
fn test_collection_into_iterator() {
    let path = DBPath::new("_rust_rocksdb_get_properties_into_iter");

    let mut opts = Options::default();
    opts.create_if_missing(true);

    let db = DB::open(&opts, &path).unwrap();

    db.put(b"key1", b"value1").unwrap();
    db.flush().unwrap();

    let collection = db.get_properties_of_all_tables().unwrap();

    // Test IntoIterator
    let count = collection.into_iter().count();
    assert!(count >= 1, "Expected at least one table");
}

/// Test that we can access numeric properties
#[test]
fn test_numeric_properties() {
    let path = DBPath::new("_rust_rocksdb_numeric_props");

    let mut opts = Options::default();
    opts.create_if_missing(true);

    let db = DB::open(&opts, &path).unwrap();

    // Write multiple entries
    for i in 0..100 {
        db.put(format!("key{:04}", i).as_bytes(), b"value").unwrap();
    }
    db.flush().unwrap();

    let collection = db.get_properties_of_all_tables().unwrap();

    for (_, props) in collection.iter() {
        // All numeric getters should work without panicking
        let _ = props.data_size();
        let _ = props.index_size();
        let _ = props.filter_size();
        let _ = props.raw_key_size();
        let _ = props.raw_value_size();
        let _ = props.num_data_blocks();
        let _ = props.num_entries();
        let _ = props.num_deletions();
        let _ = props.num_merge_operands();
        let _ = props.num_range_deletions();
    }
}

/// Test user_collected_properties and readable_properties
#[test]
fn test_property_maps() {
    let path = DBPath::new("_rust_rocksdb_property_maps");

    let mut opts = Options::default();
    opts.create_if_missing(true);

    let db = DB::open(&opts, &path).unwrap();

    db.put(b"key1", b"value1").unwrap();
    db.flush().unwrap();

    let collection = db.get_properties_of_all_tables().unwrap();

    for (_, props) in collection.iter() {
        // Get property maps - should not panic
        let user_props: HashMap<Vec<u8>, Vec<u8>> = props.user_collected_properties();
        let readable_props: HashMap<Vec<u8>, Vec<u8>> = props.readable_properties();

        // Maps should be valid (may be empty)
        let _ = user_props.len();
        let _ = readable_props.len();
    }
}

/// Test empty database behavior
#[test]
fn test_empty_database_properties() {
    let path = DBPath::new("_rust_rocksdb_empty_db_props");

    let mut opts = Options::default();
    opts.create_if_missing(true);

    let db = DB::open(&opts, &path).unwrap();

    // Don't write anything, just get properties
    let collection = db.get_properties_of_all_tables().unwrap();

    // Empty database may have 0 or some metadata tables
    // Just verify the call doesn't fail
    let _ = collection.len();
}
