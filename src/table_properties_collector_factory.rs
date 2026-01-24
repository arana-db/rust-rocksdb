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

use libc::{c_char, c_void};
use std::ffi::CStr;

use crate::{
    ffi,
    table_properties_collector::{self, TablePropertiesCollector},
};

pub trait TablePropertiesCollectorFactory {
    type Collector: TablePropertiesCollector;

    /// Returns a TablePropertiesCollector for the table build process
    fn create(&mut self, context: TablePropertiesCollectorContext) -> Self::Collector;

    /// Returns a name that identifies this table properties collector factory.
    fn name(&self) -> &CStr;
}

pub unsafe extern "C" fn destructor_callback<F>(raw_self: *mut c_void)
where
    F: TablePropertiesCollectorFactory,
{
    unsafe {
        drop(Box::from_raw(raw_self as *mut F));
    }
}

pub unsafe extern "C" fn name_callback<F>(raw_self: *mut c_void) -> *const c_char
where
    F: TablePropertiesCollectorFactory,
{
    unsafe {
        let self_ = &*(raw_self.cast_const() as *const F);
        self_.name().as_ptr()
    }
}

/// Context information passed to TablePropertiesCollectorFactory::create
pub struct TablePropertiesCollectorContext {
    /// Column family ID
    pub column_family_id: u32,
    /// Level at which the table is being created
    pub level_at_creation: i32,
    /// Total number of levels
    pub num_levels: i32,
    /// Last level inclusive max sequence number threshold
    pub last_level_inclusive_max_seqno_threshold: u64,
}

impl TablePropertiesCollectorContext {
    unsafe fn from_raw(ptr: *mut ffi::rocksdb_table_properties_collector_context_t) -> Self {
        unsafe {
            let column_family_id = 
                ffi::rocksdb_tablepropertiescollectorcontext_column_family_id(ptr);
            let level_at_creation = ffi::rocksdb_tablepropertiescollectorcontext_level_at_creation(ptr);
            let num_levels = ffi::rocksdb_tablepropertiescollectorcontext_num_levels(ptr);
            let last_level_inclusive_max_seqno_threshold = ffi::rocksdb_tablepropertiescollectorcontext_last_level_inclusive_max_seqno_threshold(ptr);

            Self {
                column_family_id,
                level_at_creation,
                num_levels,
                last_level_inclusive_max_seqno_threshold,
            }
        }
    }
}

pub unsafe extern "C" fn create_table_properties_collector_callback<F>(
    raw_self: *mut c_void,
    context: *mut ffi::rocksdb_table_properties_collector_context_t,
) -> *mut ffi::rocksdb_table_properties_collector_t
where
    F: TablePropertiesCollectorFactory,
    <F as TablePropertiesCollectorFactory>::Collector: 'static,
{
    unsafe {
        let self_ = &mut *(raw_self as *mut F);
        let context = TablePropertiesCollectorContext::from_raw(context);
        let collector = Box::new(self_.create(context));

        let collector_ptr = Box::into_raw(collector);

        ffi::rocksdb_table_properties_collector_create(
            collector_ptr as *mut c_void,
            Some(table_properties_collector::destructor_callback::<F::Collector>),
            None, // add callback (not used)
            Some(table_properties_collector::add_user_key_callback::<F::Collector>),
            None, // block_add callback (optional)
            Some(table_properties_collector::finish_callback::<F::Collector>),
            None, // get_readable_properties callback (optional)
            Some(table_properties_collector::name_callback::<F::Collector>),
            None, // need_compact callback (optional)
        )
    }
}
