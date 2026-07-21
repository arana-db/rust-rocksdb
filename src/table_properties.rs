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

use std::{collections::HashMap, marker::PhantomData, ptr::NonNull, slice};

use crate::{Error, ffi};

/// The table properties for every SST file in a column family.
pub struct TablePropertiesCollection {
    inner: NonNull<ffi::rust_rocksdb_table_properties_collection_t>,
}

impl TablePropertiesCollection {
    pub(crate) unsafe fn from_raw(
        inner: *mut ffi::rust_rocksdb_table_properties_collection_t,
    ) -> Result<Self, Error> {
        NonNull::new(inner)
            .map(|inner| Self { inner })
            .ok_or_else(|| {
                Error::new("RocksDB returned a null table properties collection".to_owned())
            })
    }

    /// Returns the number of SST files represented by this collection.
    pub fn len(&self) -> usize {
        unsafe { ffi::rust_rocksdb_table_properties_collection_len(self.inner.as_ptr()) }
    }

    /// Returns `true` when the column family has no SST files.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterates over file names and their table properties.
    ///
    /// The iterator cannot outlive the collection.
    ///
    /// ```compile_fail
    /// use rust_rocksdb::{TablePropertiesCollection, TablePropertiesCollectionIter};
    ///
    /// fn force_static(
    ///     collection: &TablePropertiesCollection,
    /// ) -> TablePropertiesCollectionIter<'static> {
    ///     collection.iter()
    /// }
    /// ```
    pub fn iter(&self) -> TablePropertiesCollectionIter<'_> {
        let inner = unsafe {
            ffi::rust_rocksdb_table_properties_collection_iter_create(self.inner.as_ptr())
        };
        TablePropertiesCollectionIter {
            inner: NonNull::new(inner).expect("RocksDB returned a null table properties iterator"),
            _collection: PhantomData,
        }
    }
}

impl Drop for TablePropertiesCollection {
    fn drop(&mut self) {
        unsafe {
            ffi::rust_rocksdb_table_properties_collection_destroy(self.inner.as_ptr());
        }
    }
}

unsafe fn copy_bytes(ptr: *const libc::c_char, len: usize) -> Vec<u8> {
    if len == 0 {
        return Vec::new();
    }
    assert!(
        !ptr.is_null(),
        "RocksDB returned a null pointer with non-zero length"
    );
    unsafe { slice::from_raw_parts(ptr.cast::<u8>(), len) }.to_vec()
}

unsafe fn next_table_properties(
    inner: NonNull<ffi::rust_rocksdb_table_properties_collection_iter_t>,
) -> Option<(Box<[u8]>, TableProperties)> {
    let mut file_name = std::ptr::null();
    let mut file_name_len = 0;
    let mut properties = std::ptr::null_mut();

    let has_next = unsafe {
        ffi::rust_rocksdb_table_properties_collection_iter_next(
            inner.as_ptr(),
            &raw mut file_name,
            &raw mut file_name_len,
            &raw mut properties,
        )
    };

    if has_next == 0 {
        return None;
    }

    let file_name = unsafe { copy_bytes(file_name, file_name_len) }.into_boxed_slice();
    let properties = unsafe { TableProperties::from_raw(properties) };
    Some((file_name, properties))
}

/// A borrowed iterator over a [`TablePropertiesCollection`].
pub struct TablePropertiesCollectionIter<'a> {
    inner: NonNull<ffi::rust_rocksdb_table_properties_collection_iter_t>,
    _collection: PhantomData<&'a TablePropertiesCollection>,
}

impl Iterator for TablePropertiesCollectionIter<'_> {
    type Item = (Box<[u8]>, TableProperties);

    fn next(&mut self) -> Option<Self::Item> {
        unsafe { next_table_properties(self.inner) }
    }
}

impl Drop for TablePropertiesCollectionIter<'_> {
    fn drop(&mut self) {
        unsafe {
            ffi::rust_rocksdb_table_properties_collection_iter_destroy(self.inner.as_ptr());
        }
    }
}

impl<'a> IntoIterator for &'a TablePropertiesCollection {
    type Item = (Box<[u8]>, TableProperties);
    type IntoIter = TablePropertiesCollectionIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// An owning iterator over a [`TablePropertiesCollection`].
pub struct TablePropertiesCollectionIntoIter {
    inner: NonNull<ffi::rust_rocksdb_table_properties_collection_iter_t>,
    _collection: TablePropertiesCollection,
}

impl Iterator for TablePropertiesCollectionIntoIter {
    type Item = (Box<[u8]>, TableProperties);

    fn next(&mut self) -> Option<Self::Item> {
        unsafe { next_table_properties(self.inner) }
    }
}

impl Drop for TablePropertiesCollectionIntoIter {
    fn drop(&mut self) {
        unsafe {
            ffi::rust_rocksdb_table_properties_collection_iter_destroy(self.inner.as_ptr());
        }
    }
}

impl IntoIterator for TablePropertiesCollection {
    type Item = (Box<[u8]>, TableProperties);
    type IntoIter = TablePropertiesCollectionIntoIter;

    fn into_iter(self) -> Self::IntoIter {
        let inner = unsafe {
            ffi::rust_rocksdb_table_properties_collection_iter_create(self.inner.as_ptr())
        };
        TablePropertiesCollectionIntoIter {
            inner: NonNull::new(inner).expect("RocksDB returned a null table properties iterator"),
            _collection: self,
        }
    }
}

/// Properties collected by RocksDB while creating one SST file.
pub struct TableProperties {
    inner: NonNull<ffi::rust_rocksdb_table_properties_t>,
}

impl TableProperties {
    unsafe fn from_raw(inner: *mut ffi::rust_rocksdb_table_properties_t) -> Self {
        Self {
            inner: NonNull::new(inner).expect("RocksDB returned a null table properties value"),
        }
    }

    pub fn data_size(&self) -> u64 {
        unsafe { ffi::rust_rocksdb_table_properties_data_size(self.inner.as_ptr()) }
    }

    pub fn index_size(&self) -> u64 {
        unsafe { ffi::rust_rocksdb_table_properties_index_size(self.inner.as_ptr()) }
    }

    pub fn filter_size(&self) -> u64 {
        unsafe { ffi::rust_rocksdb_table_properties_filter_size(self.inner.as_ptr()) }
    }

    pub fn raw_key_size(&self) -> u64 {
        unsafe { ffi::rust_rocksdb_table_properties_raw_key_size(self.inner.as_ptr()) }
    }

    pub fn raw_value_size(&self) -> u64 {
        unsafe { ffi::rust_rocksdb_table_properties_raw_value_size(self.inner.as_ptr()) }
    }

    pub fn num_data_blocks(&self) -> u64 {
        unsafe { ffi::rust_rocksdb_table_properties_num_data_blocks(self.inner.as_ptr()) }
    }

    pub fn num_entries(&self) -> u64 {
        unsafe { ffi::rust_rocksdb_table_properties_num_entries(self.inner.as_ptr()) }
    }

    pub fn num_deletions(&self) -> u64 {
        unsafe { ffi::rust_rocksdb_table_properties_num_deletions(self.inner.as_ptr()) }
    }

    pub fn num_merge_operands(&self) -> u64 {
        unsafe { ffi::rust_rocksdb_table_properties_num_merge_operands(self.inner.as_ptr()) }
    }

    pub fn num_range_deletions(&self) -> u64 {
        unsafe { ffi::rust_rocksdb_table_properties_num_range_deletions(self.inner.as_ptr()) }
    }

    fn collect_properties(
        iterator: *mut ffi::rust_rocksdb_user_collected_properties_iter_t,
    ) -> HashMap<Vec<u8>, Vec<u8>> {
        let iterator = unsafe { UserCollectedPropertiesIter::from_raw(iterator) };
        let mut result = HashMap::new();

        loop {
            let mut key = std::ptr::null();
            let mut key_len = 0;
            let mut value = std::ptr::null();
            let mut value_len = 0;
            let has_next = unsafe {
                ffi::rust_rocksdb_user_collected_properties_iter_next(
                    iterator.inner.as_ptr(),
                    &raw mut key,
                    &raw mut key_len,
                    &raw mut value,
                    &raw mut value_len,
                )
            };
            if has_next == 0 {
                break;
            }

            let key = unsafe { copy_bytes(key, key_len) };
            let value = unsafe { copy_bytes(value, value_len) };
            result.insert(key, value);
        }

        result
    }

    pub fn user_collected_properties(&self) -> HashMap<Vec<u8>, Vec<u8>> {
        let iterator = unsafe {
            ffi::rust_rocksdb_table_properties_user_collected_properties_iter_create(
                self.inner.as_ptr(),
            )
        };
        Self::collect_properties(iterator)
    }

    pub fn readable_properties(&self) -> HashMap<Vec<u8>, Vec<u8>> {
        let iterator = unsafe {
            ffi::rust_rocksdb_table_properties_readable_properties_iter_create(self.inner.as_ptr())
        };
        Self::collect_properties(iterator)
    }
}

impl Drop for TableProperties {
    fn drop(&mut self) {
        unsafe {
            ffi::rust_rocksdb_table_properties_destroy(self.inner.as_ptr());
        }
    }
}

struct UserCollectedPropertiesIter {
    inner: NonNull<ffi::rust_rocksdb_user_collected_properties_iter_t>,
}

impl UserCollectedPropertiesIter {
    unsafe fn from_raw(inner: *mut ffi::rust_rocksdb_user_collected_properties_iter_t) -> Self {
        Self {
            inner: NonNull::new(inner).expect("RocksDB returned a null user properties iterator"),
        }
    }
}

impl Drop for UserCollectedPropertiesIter {
    fn drop(&mut self) {
        unsafe {
            ffi::rust_rocksdb_user_collected_properties_iter_destroy(self.inner.as_ptr());
        }
    }
}
