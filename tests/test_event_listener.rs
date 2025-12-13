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

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;
use rocksdb::{DB, EventListener, FlushOptions, FlushJobInfo, Options, new_event_listener};
use util::DBPath;

struct TestEventListener {
    flush_begin_count: Arc<AtomicI32>,
    flush_completed_count: Arc<AtomicI32>,
}

impl EventListener for TestEventListener {
    fn on_flush_begin(&self, info: &FlushJobInfo) {
        println!("Flush file: {}", info.file_path());
        self.flush_begin_count.fetch_add(1, Ordering::Relaxed);
    }

    fn on_flush_completed(&self, _: &FlushJobInfo) {
        self.flush_completed_count.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn test_flush_callback() {
    let path = DBPath::new("_test_flush_callback");
    let flush_begin_count = Arc::new(AtomicI32::new(0));
    let flush_completed_count = Arc::new(AtomicI32::new(0));
    {
        let mut opts = Options::default();
        opts.create_if_missing(true);

        let listener = TestEventListener {
            flush_begin_count: flush_begin_count.clone(),
            flush_completed_count: flush_completed_count.clone(),
        };
        let listener_ptr = new_event_listener(listener);
        opts.add_event_listener(listener_ptr);
        let db = DB::open(&opts, &path).unwrap();
        db.put(b"k1", b"v1").unwrap();

        let mut flush_opts = FlushOptions::default();
        flush_opts.set_wait(true);
        db.flush_opt(&flush_opts).unwrap();

        // Verify callback was called
        assert_eq!(flush_begin_count.load(Ordering::Relaxed), 1);
        assert_eq!(flush_completed_count.load(Ordering::Relaxed), 1);
    }
}
