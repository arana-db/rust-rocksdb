// Copyright 2020 Tyler Neely, Alex Regueiro
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

// /* Event listener */
// typedef void (*on_flush_begin_cb)(void*, rocksdb_t*,
//     const rocksdb_flushjobinfo_t*);
// typedef void (*on_flush_completed_cb)(void*, rocksdb_t*,
//         const rocksdb_flushjobinfo_t*);

use crate::ffi;

use libc::{c_void, size_t};

pub struct FlushJobInfo {
    pub(crate) inner: *const ffi::rocksdb_flushjobinfo_t,
}

impl FlushJobInfo {
    pub fn cf_name(&self) -> String {
        unsafe {
            let mut size: size_t = 0;
            let ptr = ffi::rocksdb_flushjobinfo_cf_name(self.inner, &mut size);
            let slice = std::slice::from_raw_parts(ptr as *const u8, size);
            String::from_utf8_lossy(slice).into_owned()
        }
    }

    pub fn file_path(&self) -> String {
        unsafe {
            let mut size: size_t = 0;
            let ptr = ffi::rocksdb_flushjobinfo_file_path(self.inner, &mut size);
            let slice = std::slice::from_raw_parts(ptr as *const u8, size);
            String::from_utf8_lossy(slice).into_owned()
        }
    }

    pub fn triggered_writes_slowdown(&self) -> bool {
        unsafe { ffi::rocksdb_flushjobinfo_triggered_writes_slowdown(self.inner) != 0 }
    }

    pub fn triggered_writes_stop(&self) -> bool {
        unsafe { ffi::rocksdb_flushjobinfo_triggered_writes_stop(self.inner) != 0 }
    }

    pub fn largest_seqno(&self) -> u64 {
        unsafe { ffi::rocksdb_flushjobinfo_largest_seqno(self.inner) }
    }

    pub fn smallest_seqno(&self) -> u64 {
        unsafe { ffi::rocksdb_flushjobinfo_smallest_seqno(self.inner) }
    }
}

// WARNING: If an EventListener implementation panics, the panic will unwind across the C/FFI boundary,
// which is undefined behavior in Rust. Consider using std::panic::catch_unwind to wrap the callback body
// to prevent panic propagation to C code. Not fixing this issue for now.
pub trait EventListener: Send + Sync {
    fn on_flush_begin(&self, _: &FlushJobInfo) {}
    fn on_flush_completed(&self, _: &FlushJobInfo) {}
    // fn on_compaction_begin(&self, _: &CompactionJobInfo) {}
    // fn on_compaction_completed(&self, _: &CompactionJobInfo) {}
    // fn on_subcompaction_begin(&self, _: &SubcompactionJobInfo) {}
    // fn on_subcompaction_completed(&self, _: &SubcompactionJobInfo) {}
    // fn on_external_file_ingested(&self, _: &IngestionInfo) {}
    // fn on_background_error(&self, _: DBBackgroundErrorReason, _: MutableStatus) {}
    // fn on_stall_conditions_changed(&self, _: &WriteStallInfo) {}
    // fn on_memtable_sealed(&self, _: &MemTableInfo) {}
}

unsafe extern "C" fn destructor<E: EventListener>(ctx: *mut c_void) {
    let _ = Box::from_raw(ctx as *mut E);
}

unsafe extern "C" fn on_flush_begin<E: EventListener>(
    ctx: *mut c_void,
    _db: *mut ffi::rocksdb_t,
    flush_job_info: *const ffi::rocksdb_flushjobinfo_t,
) {
    let listener = &*(ctx as *const E);
    let info = FlushJobInfo {
        inner: flush_job_info,
    };
    listener.on_flush_begin(&info);
}

unsafe extern "C" fn on_flush_completed<E: EventListener>(
    ctx: *mut c_void,
    _db: *mut ffi::rocksdb_t,
    flush_job_info: *const ffi::rocksdb_flushjobinfo_t,
) {
    let listener = &*(ctx as *const E);
    let info = FlushJobInfo {
        inner: flush_job_info,
    };
    listener.on_flush_completed(&info);
}

unsafe extern "C" fn on_compaction_begin<E: EventListener>(
    _ctx: *mut c_void,
    _db: *mut ffi::rocksdb_t,
    _compaction_job_info: *const ffi::rocksdb_compactionjobinfo_t,
) {
    // TODO
}

unsafe extern "C" fn on_compaction_completed<E: EventListener>(
    _ctx: *mut c_void,
    _db: *mut ffi::rocksdb_t,
    _compaction_job_info: *const ffi::rocksdb_compactionjobinfo_t,
) {
    // TODO
}

unsafe extern "C" fn on_subcompaction_begin<E: EventListener>(
    _ctx: *mut c_void,
    _sub_compaction_job_info: *const ffi::rocksdb_subcompactionjobinfo_t,
) {
    // TODO
}

unsafe extern "C" fn on_subcompaction_completed<E: EventListener>(
    _ctx: *mut c_void,
    _sub_compaction_job_info: *const ffi::rocksdb_subcompactionjobinfo_t,
) {
    // TODO
}

unsafe extern "C" fn on_external_file_ingested<E: EventListener>(
    _ctx: *mut c_void,
    _db: *mut ffi::rocksdb_t,
    _external_file_ingestion_info: *const ffi::rocksdb_externalfileingestioninfo_t,
) {
    // TODO
}

unsafe extern "C" fn on_background_error<E: EventListener>(
    _ctx: *mut c_void,
    _reason: u32,
    _status_ptr: *mut ffi::rocksdb_status_ptr_t,
) {
    // TODO
}

unsafe extern "C" fn on_stall_conditions_changed<E: EventListener>(
    _ctx: *mut c_void,
    _writestall_info: *const ffi::rocksdb_writestallinfo_t,
) {
    // TODO
}

unsafe extern "C" fn on_memtable_sealed<E: EventListener>(
    _ctx: *mut c_void,
    _info: *const ffi::rocksdb_memtableinfo_t,
) {
    // TODO
}

pub fn new_event_listener<E: EventListener>(e: E) -> *mut ffi::rocksdb_eventlistener_t {
    let p = Box::new(e);
    unsafe {
        ffi::rocksdb_eventlistener_create(
            Box::into_raw(p) as *mut c_void,
            Some(destructor::<E>),
            Some(on_flush_begin::<E>),
            Some(on_flush_completed::<E>),
            Some(on_compaction_begin::<E>),
            Some(on_compaction_completed::<E>),
            Some(on_subcompaction_begin::<E>),
            Some(on_subcompaction_completed::<E>),
            Some(on_external_file_ingested::<E>),
            Some(on_background_error::<E>),
            Some(on_stall_conditions_changed::<E>),
            Some(on_memtable_sealed::<E>),
        )
    }
}
