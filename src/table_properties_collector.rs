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

use std::{
    collections::HashMap,
    ffi::{CStr, CString},
    panic::{AssertUnwindSafe, catch_unwind},
    slice,
};

use libc::{c_char, c_void, size_t};

use crate::ffi;

/// Entry type for key-value pairs observed while RocksDB builds an SST file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
            0 => Self::Put,
            1 => Self::Delete,
            2 => Self::SingleDelete,
            3 => Self::Merge,
            4 => Self::RangeDeletion,
            5 => Self::BlobIndex,
            6 => Self::DeleteWithTimestamp,
            7 => Self::WideColumnEntity,
            8 => Self::TimedPut,
            _ => Self::Other,
        }
    }
}

/// Collects application-defined properties for one RocksDB SST file.
///
/// RocksDB owns each collector after it is created and may run it on a
/// background thread. Calls for one collector are sequential and mutable, so
/// implementations need to be [`Send`] but do not need to be [`Sync`].
pub trait TablePropertiesCollector: Send + 'static {
    /// Returns a stable name used by RocksDB for diagnostics.
    fn name(&self) -> &CStr;

    /// Observes a key-value entry being added to the table.
    fn add(
        &mut self,
        key: &[u8],
        value: &[u8],
        entry_type: DBEntryType,
        sequence: u64,
        file_size: u64,
    );

    /// Produces the binary user-collected properties stored in the SST file.
    fn finish(&mut self) -> HashMap<Vec<u8>, Vec<u8>>;

    /// Produces optional human-readable properties for diagnostics.
    fn get_readable_properties(&self) -> HashMap<Vec<u8>, Vec<u8>> {
        HashMap::new()
    }
}

pub trait TablePropertiesCollectorAddUserKeyFn:
    FnMut(&[u8], &[u8], DBEntryType, u64, u64) + Send + 'static
{
}

impl<F> TablePropertiesCollectorAddUserKeyFn for F where
    F: FnMut(&[u8], &[u8], DBEntryType, u64, u64) + Send + 'static
{
}

pub trait TablePropertiesCollectorFinishFn:
    FnMut() -> HashMap<Vec<u8>, Vec<u8>> + Send + 'static
{
}

impl<F> TablePropertiesCollectorFinishFn for F where
    F: FnMut() -> HashMap<Vec<u8>, Vec<u8>> + Send + 'static
{
}

pub trait TablePropertiesCollectorGetReadablePropertiesFn:
    Fn() -> HashMap<Vec<u8>, Vec<u8>> + Send + 'static
{
}

impl<F> TablePropertiesCollectorGetReadablePropertiesFn for F where
    F: Fn() -> HashMap<Vec<u8>, Vec<u8>> + Send + 'static
{
}

/// Closure-based collector retained for compatibility with the original
/// Arana extension API.
pub struct TablePropertiesCollectorCallback<F, A, R>
where
    F: TablePropertiesCollectorFinishFn,
    A: TablePropertiesCollectorAddUserKeyFn,
    R: TablePropertiesCollectorGetReadablePropertiesFn,
{
    pub name: CString,
    pub add_fn: A,
    pub finish_fn: F,
    pub get_readable_fn: R,
}

impl<F, A, R> TablePropertiesCollector for TablePropertiesCollectorCallback<F, A, R>
where
    F: TablePropertiesCollectorFinishFn,
    A: TablePropertiesCollectorAddUserKeyFn,
    R: TablePropertiesCollectorGetReadablePropertiesFn,
{
    fn name(&self) -> &CStr {
        self.name.as_c_str()
    }

    fn add(
        &mut self,
        key: &[u8],
        value: &[u8],
        entry_type: DBEntryType,
        sequence: u64,
        file_size: u64,
    ) {
        (self.add_fn)(key, value, entry_type, sequence, file_size);
    }

    fn finish(&mut self) -> HashMap<Vec<u8>, Vec<u8>> {
        (self.finish_fn)()
    }

    fn get_readable_properties(&self) -> HashMap<Vec<u8>, Vec<u8>> {
        (self.get_readable_fn)()
    }
}

unsafe fn bytes_from_raw<'a>(data: *const c_char, len: size_t) -> Option<&'a [u8]> {
    if data.is_null() {
        return (len == 0).then_some(&[]);
    }

    // SAFETY: The caller guarantees that a non-null pointer addresses `len`
    // bytes for the duration of the synchronous callback.
    Some(unsafe { slice::from_raw_parts(data.cast::<u8>(), len) })
}

pub(crate) unsafe extern "C" fn destructor_callback<T>(state: *mut c_void)
where
    T: TablePropertiesCollector,
{
    if state.is_null() {
        return;
    }

    // SAFETY: C++ invokes this exactly once with the Box pointer transferred
    // when the collector adapter was created.
    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        drop(Box::from_raw(state.cast::<T>()));
    }));
}

pub(crate) unsafe extern "C" fn add_callback<T>(
    state: *mut c_void,
    key: *const c_char,
    key_len: size_t,
    value: *const c_char,
    value_len: size_t,
    entry_type: u8,
    sequence: u64,
    file_size: u64,
) -> u8
where
    T: TablePropertiesCollector,
{
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: C++ owns this Box for the callback lifetime and serializes
        // mutable callbacks for one collector instance.
        let Some(collector) = (unsafe { state.cast::<T>().as_mut() }) else {
            return false;
        };
        // SAFETY: C++ supplies length-delimited RocksDB slices that remain
        // valid until this callback returns.
        let Some(key) = (unsafe { bytes_from_raw(key, key_len) }) else {
            return false;
        };
        // SAFETY: Same callback-lifetime guarantee as `key` above.
        let Some(value) = (unsafe { bytes_from_raw(value, value_len) }) else {
            return false;
        };

        collector.add(
            key,
            value,
            DBEntryType::from(entry_type),
            sequence,
            file_size,
        );
        true
    }))
    .map_or(0, u8::from)
}

pub(crate) unsafe extern "C" fn finish_callback<T>(
    state: *mut c_void,
    sink: *mut ffi::rust_rocksdb_user_collected_properties_sink_t,
) -> u8
where
    T: TablePropertiesCollector,
{
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: C++ owns this Box and Finish is a serialized mutable
        // callback for the collector.
        let Some(collector) = (unsafe { state.cast::<T>().as_mut() }) else {
            return false;
        };
        if sink.is_null() {
            return false;
        }

        collector.finish().into_iter().all(|(key, value)| unsafe {
            // SAFETY: `sink` is valid only for this synchronous Finish call;
            // the native side copies both byte slices before returning.
            ffi::rust_rocksdb_user_collected_properties_sink_add(
                sink,
                key.as_ptr().cast::<c_char>(),
                key.len(),
                value.as_ptr().cast::<c_char>(),
                value.len(),
            ) != 0
        })
    }))
    .map_or(0, u8::from)
}

pub(crate) unsafe extern "C" fn readable_callback<T>(
    state: *const c_void,
    sink: *mut ffi::rust_rocksdb_user_collected_properties_sink_t,
) -> u8
where
    T: TablePropertiesCollector,
{
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: C++ owns this Box for the callback lifetime and provides
        // shared access for the readable-properties callback.
        let Some(collector) = (unsafe { state.cast::<T>().as_ref() }) else {
            return false;
        };
        if sink.is_null() {
            return false;
        }

        collector
            .get_readable_properties()
            .into_iter()
            .all(|(key, value)| unsafe {
                // SAFETY: The native sink synchronously copies the provided
                // byte slices and does not retain their pointers.
                ffi::rust_rocksdb_user_collected_properties_sink_add(
                    sink,
                    key.as_ptr().cast::<c_char>(),
                    key.len(),
                    value.as_ptr().cast::<c_char>(),
                    value.len(),
                ) != 0
            })
    }))
    .map_or(0, u8::from)
}
