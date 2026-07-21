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

use std::ffi::CStr;

use crate::table_properties_collector::TablePropertiesCollector;

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
