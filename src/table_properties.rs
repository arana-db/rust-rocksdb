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

use std::collections::HashMap;
use std::marker::PhantomData;

use crate::ffi;

/// A collection of table properties for all SST files in a column family.
pub struct TablePropertiesCollection {
    inner: *mut ffi::rocksdb_table_properties_collection_t,
}

impl TablePropertiesCollection {
    /// Creates a new TablePropertiesCollection from a raw pointer.
    ///
    /// # Safety
    /// The pointer must be a valid rocksdb_table_properties_collection_t pointer
    /// returned by rocksdb_get_properties_of_all_tables or rocksdb_get_properties_of_all_tables_cf.
    pub(crate) unsafe fn from_raw(inner: *mut ffi::rocksdb_table_properties_collection_t) -> Self {
        Self { inner }
    }

    /// Returns the number of tables in the collection.
    pub fn len(&self) -> usize {
        unsafe { ffi::rocksdb_table_properties_collection_len(self.inner) }
    }

    /// Returns true if the collection is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns an iterator over the table properties in the collection.
    pub fn iter(&self) -> TablePropertiesCollectionIter<'_> {
        unsafe {
            let iter = ffi::rocksdb_table_properties_collection_iter_create(self.inner);
            TablePropertiesCollectionIter {
                inner: iter,
                _phantom: PhantomData,
            }
        }
    }
}

impl Drop for TablePropertiesCollection {
    fn drop(&mut self) {
        unsafe {
            ffi::rocksdb_table_properties_collection_destroy(self.inner);
        }
    }
}

unsafe impl Send for TablePropertiesCollection {}
unsafe impl Sync for TablePropertiesCollection {}

/// An iterator over table properties in a collection.
pub struct TablePropertiesCollectionIter<'a> {
    inner: *mut ffi::rocksdb_table_properties_collection_iter_t,
    _phantom: PhantomData<&'a TablePropertiesCollection>,
}

impl<'a> Iterator for TablePropertiesCollectionIter<'a> {
    type Item = (String, TableProperties);

    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            let mut key: *const libc::c_char = std::ptr::null();
            let mut key_len: libc::size_t = 0;
            let mut props: *mut ffi::rocksdb_table_properties_t = std::ptr::null_mut();

            if ffi::rocksdb_table_properties_collection_iter_next(
                self.inner,
                &mut key,
                &mut key_len,
                &mut props,
            ) {
                let key_slice = std::slice::from_raw_parts(key as *const u8, key_len);
                let key_string = String::from_utf8_lossy(key_slice).into_owned();
                let table_props = TableProperties::from_raw(props);
                Some((key_string, table_props))
            } else {
                None
            }
        }
    }
}

impl<'a> Drop for TablePropertiesCollectionIter<'a> {
    fn drop(&mut self) {
        unsafe {
            ffi::rocksdb_table_properties_collection_iter_destroy(self.inner);
        }
    }
}

/// An owning iterator that consumes the collection.
pub struct TablePropertiesCollectionIntoIter {
    _collection: TablePropertiesCollection,
    inner: *mut ffi::rocksdb_table_properties_collection_iter_t,
}

impl Iterator for TablePropertiesCollectionIntoIter {
    type Item = (String, TableProperties);

    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            let mut key: *const libc::c_char = std::ptr::null();
            let mut key_len: libc::size_t = 0;
            let mut props: *mut ffi::rocksdb_table_properties_t = std::ptr::null_mut();

            if ffi::rocksdb_table_properties_collection_iter_next(
                self.inner,
                &mut key,
                &mut key_len,
                &mut props,
            ) {
                let key_slice = std::slice::from_raw_parts(key as *const u8, key_len);
                let key_string = String::from_utf8_lossy(key_slice).into_owned();
                let table_props = TableProperties::from_raw(props);
                Some((key_string, table_props))
            } else {
                None
            }
        }
    }
}

impl Drop for TablePropertiesCollectionIntoIter {
    fn drop(&mut self) {
        unsafe {
            ffi::rocksdb_table_properties_collection_iter_destroy(self.inner);
        }
    }
}

impl IntoIterator for TablePropertiesCollection {
    type Item = (String, TableProperties);
    type IntoIter = TablePropertiesCollectionIntoIter;

    fn into_iter(mut self) -> Self::IntoIter {
        unsafe {
            let iter = ffi::rocksdb_table_properties_collection_iter_create(self.inner);
            // We need to keep the collection alive, so we use std::mem::swap
            // to move it into the iterator
            let collection = std::mem::replace(&mut self, std::mem::zeroed());
            std::mem::forget(self); // Prevent double-drop
            TablePropertiesCollectionIntoIter {
                _collection: collection,
                inner: iter,
            }
        }
    }
}

/// Properties for a single SST file.
pub struct TableProperties {
    inner: *mut ffi::rocksdb_table_properties_t,
}

impl TableProperties {
    /// Creates a new TableProperties from a raw pointer.
    ///
    /// # Safety
    /// The pointer must be a valid rocksdb_table_properties_t pointer
    /// returned by the iterator's next() method.
    pub(crate) unsafe fn from_raw(inner: *mut ffi::rocksdb_table_properties_t) -> Self {
        Self { inner }
    }

    /// Returns the raw data size.
    pub fn data_size(&self) -> u64 {
        unsafe { ffi::rocksdb_table_properties_get_data_size(self.inner) }
    }

    /// Returns the index size.
    pub fn index_size(&self) -> u64 {
        unsafe { ffi::rocksdb_table_properties_get_index_size(self.inner) }
    }

    /// Returns the filter size.
    pub fn filter_size(&self) -> u64 {
        unsafe { ffi::rocksdb_table_properties_get_filter_size(self.inner) }
    }

    /// Returns the raw key size.
    pub fn raw_key_size(&self) -> u64 {
        unsafe { ffi::rocksdb_table_properties_get_raw_key_size(self.inner) }
    }

    /// Returns the raw value size.
    pub fn raw_value_size(&self) -> u64 {
        unsafe { ffi::rocksdb_table_properties_get_raw_value_size(self.inner) }
    }

    /// Returns the number of data blocks.
    pub fn num_data_blocks(&self) -> u64 {
        unsafe { ffi::rocksdb_table_properties_get_num_data_blocks(self.inner) }
    }

    /// Returns the number of entries.
    pub fn num_entries(&self) -> u64 {
        unsafe { ffi::rocksdb_table_properties_get_num_entries(self.inner) }
    }

    /// Returns the number of deletions.
    pub fn num_deletions(&self) -> u64 {
        unsafe { ffi::rocksdb_table_properties_get_num_deletions(self.inner) }
    }

    /// Returns the number of merge operands.
    pub fn num_merge_operands(&self) -> u64 {
        unsafe { ffi::rocksdb_table_properties_get_num_merge_operands(self.inner) }
    }

    /// Returns the number of range deletions.
    pub fn num_range_deletions(&self) -> u64 {
        unsafe { ffi::rocksdb_table_properties_get_num_range_deletions(self.inner) }
    }

    /// Returns user-collected properties as a HashMap.
    pub fn user_collected_properties(&self) -> HashMap<Vec<u8>, Vec<u8>> {
        unsafe {
            let props = ffi::rocksdb_table_properties_get_user_collected_properties(self.inner);
            self.collect_properties_from_ptr(props)
        }
    }

    /// Returns readable properties as a HashMap.
    pub fn readable_properties(&self) -> HashMap<Vec<u8>, Vec<u8>> {
        unsafe {
            let props = ffi::rocksdb_table_properties_get_readable_properties(self.inner);
            self.collect_properties_from_ptr(props)
        }
    }

    unsafe fn collect_properties_from_ptr(
        &self,
        props: *const ffi::rocksdb_user_collected_properties_t,
    ) -> HashMap<Vec<u8>, Vec<u8>> {
        if props.is_null() {
            return HashMap::new();
        }

        let mut map = HashMap::new();
        let iter = ffi::rocksdb_user_collected_properties_iter_create(props);

        loop {
            let mut key: *const libc::c_char = std::ptr::null();
            let mut key_len: libc::size_t = 0;
            let mut val: *const libc::c_char = std::ptr::null();
            let mut val_len: libc::size_t = 0;

            if !ffi::rocksdb_user_collected_properties_iter_next(
                iter,
                &mut key,
                &mut key_len,
                &mut val,
                &mut val_len,
            ) {
                break;
            }

            let key_vec = std::slice::from_raw_parts(key as *const u8, key_len).to_vec();
            let val_vec = std::slice::from_raw_parts(val as *const u8, val_len).to_vec();
            map.insert(key_vec, val_vec);
        }

        ffi::rocksdb_user_collected_properties_iter_destroy(iter);
        map
    }
}

impl Drop for TableProperties {
    fn drop(&mut self) {
        unsafe {
            ffi::rocksdb_table_properties_destroy(self.inner);
        }
    }
}

unsafe impl Send for TableProperties {}
unsafe impl Sync for TableProperties {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DB, Options};
    use tempfile::Builder;

    #[test]
    fn test_table_properties_basic() {
        let temp_dir = Builder::new()
            .prefix("_rust_rocksdb_table_properties_test")
            .tempdir()
            .expect("Failed to create temp dir");

        let mut opts = Options::default();
        opts.create_if_missing(true);

        let db = DB::open(&opts, temp_dir.path()).unwrap();

        // Write some data
        db.put(b"key1", b"value1").unwrap();
        db.put(b"key2", b"value2").unwrap();
        db.put(b"key3", b"value3").unwrap();

        // Flush to create SST files
        db.flush().unwrap();

        // Get properties (will be implemented in db.rs Task 5)
        // This is a placeholder for the actual test
        // let props = db.get_properties_of_all_tables().unwrap();
        // assert!(!props.is_empty());
    }
}
