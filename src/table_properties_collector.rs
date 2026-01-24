// Copyright (c) 2024 rust-rocksdb contributors
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

use libc::{c_char, c_void, size_t};
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::slice;

use crate::ffi;

/// Entry type for key-value pairs in RocksDB
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum DBEntryType {
    Put = 0,
    Delete = 1,
    SingleDelete = 2,
    Merge = 3,
    RangeDeletion = 4,
    BlobIndex = 5,
    DeleteWithTimestamp = 6,
    WideColumnEntity = 7,
    TimedPut = 8,
    Other = 9,
}

impl From<u8> for DBEntryType {
    fn from(value: u8) -> Self {
        match value {
            0 => DBEntryType::Put,
            1 => DBEntryType::Delete,
            2 => DBEntryType::SingleDelete,
            3 => DBEntryType::Merge,
            4 => DBEntryType::RangeDeletion,
            5 => DBEntryType::BlobIndex,
            6 => DBEntryType::DeleteWithTimestamp,
            7 => DBEntryType::WideColumnEntity,
            8 => DBEntryType::TimedPut,
            _ => DBEntryType::Other,
        }
    }
}

pub trait TablePropertiesCollector {
    /// Returns a name that identifies this table properties collector.
    /// The name will be printed to LOG file on start up for diagnosis.
    fn name(&self) -> &CStr;
    /// Will be called when a new key/value pair is inserted into the table.
    fn add(&mut self, key: &[u8], value: &[u8], entry_type: DBEntryType, seq: u64, file_size: u64);
    /// Will be called when a table has already been built and is ready for
    /// writing the properties block.
    fn finish(&mut self, ) -> HashMap<Vec<u8>, Vec<u8>>;
}

pub trait TablePropertiesCollectorAddUserKeyFn: FnMut(&[u8], &[u8], DBEntryType, u64, u64) {}
impl<F> TablePropertiesCollectorAddUserKeyFn for F 
where 
    F: FnMut(&[u8], &[u8], DBEntryType, u64, u64) + Send + 'static 
{}
pub trait TablePropertiesCollectorFinishFn: FnMut() -> HashMap<Vec<u8>, Vec<u8>> {}
impl<F> TablePropertiesCollectorFinishFn for F 
where 
    F: FnMut() -> HashMap<Vec<u8>, Vec<u8>> + Send + 'static 
{}

pub struct  TablePropertiesCollectorCallback<F, A>
where 
    F : TablePropertiesCollectorFinishFn,
    A : TablePropertiesCollectorAddUserKeyFn,
{
    pub name: CString,
    pub add_fn: A,
    pub finish_fn: F,
}

impl<F, A> TablePropertiesCollector for TablePropertiesCollectorCallback<F, A>
where 
    F: TablePropertiesCollectorFinishFn,
    A: TablePropertiesCollectorAddUserKeyFn,
{
    fn name(&self) -> &CStr {
        self.name.as_c_str()
    }

    fn add(&mut self, key: &[u8], value: &[u8], entry_type: DBEntryType, seq: u64, file_size: u64) {
        (self.add_fn)(key,value,entry_type,seq,file_size)
    }

    fn finish(&mut self) -> HashMap<Vec<u8>, Vec<u8>> {
        (self.finish_fn)()
    }
}

pub unsafe extern "C" fn destructor_callback<F>(raw_cb: *mut c_void)
where 
    F: TablePropertiesCollector,
{
    unsafe {
        drop(Box::from_raw(raw_cb as *mut F));
    }
}


pub unsafe extern "C" fn name_callback<F>(raw_cb: *mut c_void) -> *const c_char
where
    F: TablePropertiesCollector,
{
    unsafe {
        let cb = &*(raw_cb as *mut F);
        cb.name().as_ptr()
    }
}

pub unsafe extern "C" fn finish_callback<T>(
    raw_self: *mut c_void,
    props: *mut ffi::rocksdb_user_collected_properties_t,
    _err: *mut *mut c_char,
) where
    T: TablePropertiesCollector,
{
    unsafe {
        let self_ = &mut *(raw_self as *mut T);
        for (key, value) in self_.finish() {
            ffi::rocksdb_user_collected_properties_add(
                props,
                key.as_ptr() as *const c_char,
                key.len(),
                value.as_ptr() as *const c_char,
                value.len(),
            );
        }
    }
}

pub unsafe extern "C" fn add_user_key_callback<T>(
    raw_self: *mut c_void,
    key: *const c_char,
    key_len: size_t,
    value: *const c_char,
    value_len: size_t,
    entry_type: *mut ffi::rocksdb_entry_type_t,
    seq: *mut ffi::rocksdb_sequence_number_t,
    file_size: u64,
    _err: *mut *mut c_char,
) where
    T: TablePropertiesCollector,
{
    unsafe {
        let self_ = &mut *(raw_self as *mut T);
        
        let key_slice = slice::from_raw_parts(key as *const u8, key_len);
        let value_slice = slice::from_raw_parts(value as *const u8, value_len);
        
        let entry_type_val = if !entry_type.is_null() {
            DBEntryType::from(*(entry_type as *const u8))
        } else {
            DBEntryType::Other
        };
        
        let seq_val = if !seq.is_null() {
            *(seq as *const u64)
        } else {
            0
        };
        
        self_.add(key_slice, value_slice, entry_type_val, seq_val, file_size);
    }
}