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
    ffi::CStr,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::null_mut,
};

use libc::c_void;

use crate::{
    ffi,
    table_properties_collector::{self, TablePropertiesCollector},
};

/// Context supplied by RocksDB when it starts building an SST file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TablePropertiesCollectorContext {
    pub column_family_id: u32,
    pub level_at_creation: i32,
    pub num_levels: i32,
    pub last_level_inclusive_max_seqno_threshold: u64,
}

/// Creates an independent table-properties collector for each SST build.
///
/// RocksDB may call a factory concurrently from background threads. Factory
/// implementations must therefore support shared concurrent access.
pub trait TablePropertiesCollectorFactory: Send + Sync + 'static {
    type Collector: TablePropertiesCollector;

    /// Creates a collector for one table build.
    fn create(&self, context: TablePropertiesCollectorContext) -> Self::Collector;

    /// Returns a stable name used by RocksDB for diagnostics.
    fn name(&self) -> &CStr;
}

pub(crate) unsafe extern "C" fn destructor_callback<F>(state: *mut c_void)
where
    F: TablePropertiesCollectorFactory,
{
    if state.is_null() {
        return;
    }

    // SAFETY: C++ invokes this exactly once with the Box pointer transferred
    // when the shared factory adapter was created.
    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        drop(Box::from_raw(state.cast::<F>()));
    }));
}

pub(crate) unsafe extern "C" fn create_callback<F>(
    state: *const c_void,
    column_family_id: u32,
    level_at_creation: i32,
    num_levels: i32,
    last_level_inclusive_max_seqno_threshold: u64,
) -> *mut ffi::rust_rocksdb_table_properties_collector_t
where
    F: TablePropertiesCollectorFactory,
{
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: The shared C++ adapter owns this Box and the Factory trait
        // requires Sync for concurrent shared access.
        let Some(factory) = (unsafe { state.cast::<F>().as_ref() }) else {
            return null_mut();
        };
        let collector = Box::new(factory.create(TablePropertiesCollectorContext {
            column_family_id,
            level_at_creation,
            num_levels,
            last_level_inclusive_max_seqno_threshold,
        }));
        let name = collector.name();
        let name_ptr = name.as_ptr();
        let name_len = name.to_bytes().len();
        let collector_state = Box::into_raw(collector).cast::<c_void>();

        // SAFETY: The collector Box is transferred to this call. The native
        // wrapper copies the name synchronously and consumes the Box state on
        // both success and failure.
        unsafe {
            ffi::rust_rocksdb_table_properties_collector_create(
                collector_state,
                Some(table_properties_collector::destructor_callback::<F::Collector>),
                name_ptr,
                name_len,
                Some(table_properties_collector::add_callback::<F::Collector>),
                Some(table_properties_collector::finish_callback::<F::Collector>),
                Some(table_properties_collector::readable_callback::<F::Collector>),
            )
        }
    }))
    .unwrap_or(null_mut())
}
