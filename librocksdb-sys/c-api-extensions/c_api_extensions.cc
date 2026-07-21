// Local additions to the RocksDB C API. See c_api_extensions.h for the
// rationale and the list of extensions; this file just defines the
// declarations from that header.
//
// Each extension is the smallest practical delta over the existing C API:
// either an option setter/getter pair or a thin wrapper over an upstream C++
// callback surface that has not reached rocksdb/c.h yet.

#include "c_api_extensions.h"

#include <cassert>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <new>
#include <string>
#include <unordered_map>
#include <utility>

#include "rocksdb/db.h"
#include "rocksdb/listener.h"
#include "rocksdb/options.h"
#include "rocksdb/table.h"
#include "rocksdb/table_properties.h"

using ROCKSDB_NAMESPACE::BackgroundErrorRecoveryInfo;
using ROCKSDB_NAMESPACE::BlockBasedTableOptions;
using ROCKSDB_NAMESPACE::ColumnFamilyHandle;
using ROCKSDB_NAMESPACE::CompactRangeOptions;
using ROCKSDB_NAMESPACE::CompactionJobInfo;
using ROCKSDB_NAMESPACE::DB;
using ROCKSDB_NAMESPACE::EventListener;
using ROCKSDB_NAMESPACE::ExternalFileIngestionInfo;
using ROCKSDB_NAMESPACE::FlushJobInfo;
using ROCKSDB_NAMESPACE::Options;
using ROCKSDB_NAMESPACE::ReadOptions;
using ROCKSDB_NAMESPACE::Status;
using ROCKSDB_NAMESPACE::SubcompactionJobInfo;
using ROCKSDB_NAMESPACE::TableProperties;
using ROCKSDB_NAMESPACE::TablePropertiesCollection;
using ROCKSDB_NAMESPACE::UserCollectedProperties;
using ROCKSDB_NAMESPACE::WriteStallInfo;
using ROCKSDB_NAMESPACE::MemTableInfo;

struct rust_rocksdb_status_t {
  Status* rep;
};

struct rust_rocksdb_background_error_recovery_info_t {
  const BackgroundErrorRecoveryInfo* rep;
};

static bool RustSaveError(char** errptr, const Status& s) {
  assert(errptr != nullptr);
  if (s.ok()) {
    return false;
  }

  std::string message = s.ToString();
  char* copy = static_cast<char*>(std::malloc(message.size() + 1));
  if (copy != nullptr) {
    std::memcpy(copy, message.c_str(), message.size() + 1);
  }

  if (*errptr != nullptr) {
    std::free(*errptr);
  }
  *errptr = copy;
  return true;
}

static void RustSaveStaticError(char** errptr, const char* message) noexcept {
  assert(errptr != nullptr);
  const size_t length = std::strlen(message);
  char* copy = static_cast<char*>(std::malloc(length + 1));
  if (copy != nullptr) {
    std::memcpy(copy, message, length + 1);
  }

  if (*errptr != nullptr) {
    std::free(*errptr);
  }
  *errptr = copy;
}

extern "C" void rust_rocksdb_status_get_error(rust_rocksdb_status_t* status,
                                               char** errptr) {
  RustSaveError(errptr, *(status->rep));
}

extern "C" unsigned char rust_rocksdb_status_get_severity(
    rust_rocksdb_status_t* status) {
  return static_cast<unsigned char>(status->rep->severity());
}

extern "C" void rust_rocksdb_status_reset(rust_rocksdb_status_t* status) {
  *(status->rep) = Status::OK();
}

extern "C" void rust_rocksdb_background_error_recovery_info_old_bg_error(
    const rust_rocksdb_background_error_recovery_info_t* info, char** errptr) {
  RustSaveError(errptr, info->rep->old_bg_error);
}

extern "C" unsigned char
rust_rocksdb_background_error_recovery_info_old_bg_error_severity(
    const rust_rocksdb_background_error_recovery_info_t* info) {
  return static_cast<unsigned char>(info->rep->old_bg_error.severity());
}

extern "C" void rust_rocksdb_background_error_recovery_info_new_bg_error(
    const rust_rocksdb_background_error_recovery_info_t* info, char** errptr) {
  RustSaveError(errptr, info->rep->new_bg_error);
}

extern "C" unsigned char
rust_rocksdb_background_error_recovery_info_new_bg_error_severity(
    const rust_rocksdb_background_error_recovery_info_t* info) {
  return static_cast<unsigned char>(info->rep->new_bg_error.severity());
}

struct rust_rocksdb_eventlistener_t : public EventListener {
  void* state{};
  void (*destructor)(void*){};
  rust_rocksdb_on_flush_begin_cb on_flush_begin{};
  rust_rocksdb_on_flush_completed_cb on_flush_completed{};
  rust_rocksdb_on_compaction_begin_cb on_compaction_begin{};
  rust_rocksdb_on_compaction_completed_cb on_compaction_completed{};
  rust_rocksdb_on_subcompaction_begin_cb on_subcompaction_begin{};
  rust_rocksdb_on_subcompaction_completed_cb on_subcompaction_completed{};
  rust_rocksdb_on_external_file_ingested_cb on_external_file_ingested{};
  rust_rocksdb_on_background_error_cb on_background_error{};
  rust_rocksdb_on_error_recovery_begin_cb on_error_recovery_begin{};
  rust_rocksdb_on_error_recovery_end_cb on_error_recovery_end{};
  rust_rocksdb_on_stall_conditions_changed_cb on_stall_conditions_changed{};
  rust_rocksdb_on_memtable_sealed_cb on_memtable_sealed{};

  rust_rocksdb_eventlistener_t() = default;

  rust_rocksdb_eventlistener_t(const rust_rocksdb_eventlistener_t&) = delete;
  rust_rocksdb_eventlistener_t& operator=(
      const rust_rocksdb_eventlistener_t&) = delete;
  rust_rocksdb_eventlistener_t(rust_rocksdb_eventlistener_t&&) = delete;
  rust_rocksdb_eventlistener_t& operator=(rust_rocksdb_eventlistener_t&&) =
      delete;

  void OnFlushBegin(DB* /*db*/, const FlushJobInfo& info) override {
    if (on_flush_begin != nullptr) {
      on_flush_begin(state,
                     reinterpret_cast<const rocksdb_flushjobinfo_t*>(&info));
    }
  }

  void OnFlushCompleted(DB* /*db*/, const FlushJobInfo& info) override {
    if (on_flush_completed != nullptr) {
      on_flush_completed(
          state, reinterpret_cast<const rocksdb_flushjobinfo_t*>(&info));
    }
  }

  void OnCompactionBegin(DB* /*db*/, const CompactionJobInfo& info) override {
    if (on_compaction_begin != nullptr) {
      on_compaction_begin(
          state, reinterpret_cast<const rocksdb_compactionjobinfo_t*>(&info));
    }
  }

  void OnCompactionCompleted(DB* /*db*/, const CompactionJobInfo& info)
      override {
    if (on_compaction_completed != nullptr) {
      on_compaction_completed(
          state, reinterpret_cast<const rocksdb_compactionjobinfo_t*>(&info));
    }
  }

  void OnSubcompactionBegin(const SubcompactionJobInfo& info) override {
    if (on_subcompaction_begin != nullptr) {
      on_subcompaction_begin(
          state,
          reinterpret_cast<const rocksdb_subcompactionjobinfo_t*>(&info));
    }
  }

  void OnSubcompactionCompleted(const SubcompactionJobInfo& info) override {
    if (on_subcompaction_completed != nullptr) {
      on_subcompaction_completed(
          state,
          reinterpret_cast<const rocksdb_subcompactionjobinfo_t*>(&info));
    }
  }

  void OnExternalFileIngested(DB* /*db*/,
                              const ExternalFileIngestionInfo& info) override {
    if (on_external_file_ingested != nullptr) {
      on_external_file_ingested(
          state,
          reinterpret_cast<const rocksdb_externalfileingestioninfo_t*>(&info));
    }
  }

  void OnBackgroundError(ROCKSDB_NAMESPACE::BackgroundErrorReason reason,
                         Status* status) override {
    if (on_background_error != nullptr) {
      rust_rocksdb_status_t s = {status};
      on_background_error(state, static_cast<uint32_t>(reason), &s);
    }
  }

  void OnErrorRecoveryBegin(ROCKSDB_NAMESPACE::BackgroundErrorReason reason,
                            Status bg_error,
                            bool* auto_recovery) override {
    if (on_error_recovery_begin != nullptr) {
      rust_rocksdb_status_t s = {&bg_error};
      unsigned char auto_recovery_value =
          auto_recovery != nullptr && *auto_recovery;
      on_error_recovery_begin(state, static_cast<uint32_t>(reason), &s,
                              &auto_recovery_value);
      if (auto_recovery != nullptr) {
        *auto_recovery = auto_recovery_value != 0;
      }
    }
    bg_error.PermitUncheckedError();
  }

  void OnErrorRecoveryEnd(const BackgroundErrorRecoveryInfo& info) override {
    if (on_error_recovery_end != nullptr) {
      rust_rocksdb_background_error_recovery_info_t c_info = {&info};
      on_error_recovery_end(state, &c_info);
    }
    info.old_bg_error.PermitUncheckedError();
    info.new_bg_error.PermitUncheckedError();
  }

  void OnStallConditionsChanged(const WriteStallInfo& info) override {
    if (on_stall_conditions_changed != nullptr) {
      on_stall_conditions_changed(
          state, reinterpret_cast<const rocksdb_writestallinfo_t*>(&info));
    }
  }

  void OnMemTableSealed(const MemTableInfo& info) override {
    if (on_memtable_sealed != nullptr) {
      on_memtable_sealed(
          state, reinterpret_cast<const rocksdb_memtableinfo_t*>(&info));
    }
  }

  ~rust_rocksdb_eventlistener_t() override {
    if (destructor != nullptr) {
      destructor(state);
    }
  }
};

extern "C" rust_rocksdb_eventlistener_t* rust_rocksdb_eventlistener_create(
    void* state, void (*destructor)(void*),
    rust_rocksdb_on_flush_begin_cb on_flush_begin,
    rust_rocksdb_on_flush_completed_cb on_flush_completed,
    rust_rocksdb_on_compaction_begin_cb on_compaction_begin,
    rust_rocksdb_on_compaction_completed_cb on_compaction_completed,
    rust_rocksdb_on_subcompaction_begin_cb on_subcompaction_begin,
    rust_rocksdb_on_subcompaction_completed_cb on_subcompaction_completed,
    rust_rocksdb_on_external_file_ingested_cb on_external_file_ingested,
    rust_rocksdb_on_background_error_cb on_background_error,
    rust_rocksdb_on_error_recovery_begin_cb on_error_recovery_begin,
    rust_rocksdb_on_error_recovery_end_cb on_error_recovery_end,
    rust_rocksdb_on_stall_conditions_changed_cb on_stall_conditions_changed,
    rust_rocksdb_on_memtable_sealed_cb on_memtable_sealed) {
  rust_rocksdb_eventlistener_t* listener = new rust_rocksdb_eventlistener_t;
  listener->state = state;
  listener->destructor = destructor;
  listener->on_flush_begin = on_flush_begin;
  listener->on_flush_completed = on_flush_completed;
  listener->on_compaction_begin = on_compaction_begin;
  listener->on_compaction_completed = on_compaction_completed;
  listener->on_subcompaction_begin = on_subcompaction_begin;
  listener->on_subcompaction_completed = on_subcompaction_completed;
  listener->on_external_file_ingested = on_external_file_ingested;
  listener->on_background_error = on_background_error;
  listener->on_error_recovery_begin = on_error_recovery_begin;
  listener->on_error_recovery_end = on_error_recovery_end;
  listener->on_stall_conditions_changed = on_stall_conditions_changed;
  listener->on_memtable_sealed = on_memtable_sealed;
  return listener;
}

extern "C" void rust_rocksdb_eventlistener_destroy(
    rust_rocksdb_eventlistener_t* listener) {
  delete listener;
}

extern "C" void rust_rocksdb_options_add_eventlistener(
    rocksdb_options_t* opt, rust_rocksdb_eventlistener_t* listener) {
  reinterpret_cast<Options*>(opt)->listeners.emplace_back(
      std::shared_ptr<EventListener>(listener));
}

// The opaque-handle types the C API hands out are defined at file scope in
// `rocksdb/db/c.cc` as POD wrappers around a single C++ class:
//
//   struct rocksdb_readoptions_t { ReadOptions rep; /* trailing Slices */ };
//   struct rocksdb_options_t { Options rep; };
//   struct rocksdb_block_based_table_options_t { BlockBasedTableOptions rep; };
//
// In every case the `rep` field is the FIRST member, so a pointer to the
// opaque C type also points at the start of its embedded C++ `rep` field.
// We exploit that here with a direct `reinterpret_cast` instead of
// replicating the struct definitions — replication would either drift
// silently if upstream ever adds a field before `rep` (the very change that
// would also break this cast), or trip C++'s one-definition rule against
// c.h's `typedef struct rocksdb_readoptions_t rocksdb_readoptions_t;`.
//
// If upstream ever adds a field BEFORE `rep` in any of these wrappers,
// every test that round-trips a value through one of our setters will
// fail loudly: the setter would write to one offset and rocksdb's
// internal code would read from another. The integration tests in
// `tests/test_rocksdb_options.rs` cover all three options that this file
// exposes, so a layout regression is detectable.

// -----------------------------------------------------------------------------
// ReadOptions::optimize_multiget_for_io
// -----------------------------------------------------------------------------

extern "C" void rocksdb_readoptions_set_optimize_multiget_for_io(
    rocksdb_readoptions_t* opt, unsigned char v) {
  reinterpret_cast<ReadOptions*>(opt)->optimize_multiget_for_io = v;
}

extern "C" unsigned char rocksdb_readoptions_get_optimize_multiget_for_io(
    rocksdb_readoptions_t* opt) {
  return reinterpret_cast<ReadOptions*>(opt)->optimize_multiget_for_io;
}

// -----------------------------------------------------------------------------
// BlockBasedTableOptions::uniform_cv_threshold
//
// The corresponding `kAuto` enum value is declared in c_api_extensions.h
// — no C-side definition is needed because the existing
// `rocksdb_block_based_options_set_index_block_search_type` setter in
// upstream `c.cc` already does `static_cast<BlockSearchType>(int)` and
// accepts any value the caller passes.
// -----------------------------------------------------------------------------

extern "C" void rocksdb_block_based_options_set_uniform_cv_threshold(
    rocksdb_block_based_table_options_t* opt, double v) {
  reinterpret_cast<BlockBasedTableOptions*>(opt)->uniform_cv_threshold = v;
}

// -----------------------------------------------------------------------------
// AdvancedColumnFamilyOptions::memtable_batch_lookup_optimization
// -----------------------------------------------------------------------------

extern "C" void rocksdb_options_set_memtable_batch_lookup_optimization(
    rocksdb_options_t* opt, unsigned char v) {
  reinterpret_cast<Options*>(opt)->memtable_batch_lookup_optimization = v;
}

extern "C" unsigned char rocksdb_options_get_memtable_batch_lookup_optimization(
    rocksdb_options_t* opt) {
  return reinterpret_cast<Options*>(opt)->memtable_batch_lookup_optimization;
}

// -----------------------------------------------------------------------------
// CompactOptions::blob_garbage_collection_age_cutoff
// -----------------------------------------------------------------------------

extern "C" void rocksdb_compactoptions_set_blob_garbage_collection_age_cutoff(
    rocksdb_compactoptions_t* opt, double v) {
  reinterpret_cast<CompactRangeOptions*>(opt)->blob_garbage_collection_age_cutoff = v;
}

extern "C" double rocksdb_compactoptions_get_blob_garbage_collection_age_cutoff(
    rocksdb_compactoptions_t* opt) {
  return reinterpret_cast<CompactRangeOptions*>(opt)->blob_garbage_collection_age_cutoff;
}

// -----------------------------------------------------------------------------
// DB::GetPropertiesOfAllTables
//
// RocksDB's db/c.cc defines rocksdb_t with DB* rep as its first field and
// rocksdb_column_family_handle_t with ColumnFamilyHandle* rep as its first
// field. The named-column-family integration test exercises both layout
// assumptions. The extension owns every C++ object it creates; it never
// exposes references into the vendored RocksDB sources as C structs.
// -----------------------------------------------------------------------------

struct rust_rocksdb_table_properties_collection_t {
  TablePropertiesCollection rep;
};

struct rust_rocksdb_table_properties_collection_iter_t {
  TablePropertiesCollection::const_iterator current;
  TablePropertiesCollection::const_iterator end;
};

struct rust_rocksdb_table_properties_t {
  std::shared_ptr<const TableProperties> rep;
};

struct rust_rocksdb_user_collected_properties_iter_t {
  UserCollectedProperties::const_iterator current;
  UserCollectedProperties::const_iterator end;
};

static DB* RustDB(rocksdb_t* db) noexcept {
  return *reinterpret_cast<DB**>(db);
}

static ColumnFamilyHandle* RustColumnFamilyHandle(
    rocksdb_column_family_handle_t* column_family) noexcept {
  return *reinterpret_cast<ColumnFamilyHandle**>(column_family);
}

template <typename T, typename... Args>
static T* RustNewOrAbort(Args&&... args) noexcept {
  T* value = new (std::nothrow) T{std::forward<Args>(args)...};
  if (value == nullptr) {
    std::abort();
  }
  return value;
}

extern "C" rust_rocksdb_table_properties_collection_t*
rust_rocksdb_get_properties_of_all_tables(rocksdb_t* db,
                                           char** errptr) noexcept {
  auto* raw =
      new (std::nothrow) rust_rocksdb_table_properties_collection_t();
  if (raw == nullptr) {
    RustSaveStaticError(errptr,
                        "failed to allocate table properties collection");
    return nullptr;
  }

  std::unique_ptr<rust_rocksdb_table_properties_collection_t> collection(raw);
  try {
    Status status = RustDB(db)->GetPropertiesOfAllTables(&collection->rep);
    if (RustSaveError(errptr, status)) {
      return nullptr;
    }
    return collection.release();
  } catch (...) {
    RustSaveStaticError(errptr, "exception while reading table properties");
    return nullptr;
  }
}

extern "C" rust_rocksdb_table_properties_collection_t*
rust_rocksdb_get_properties_of_all_tables_cf(
    rocksdb_t* db, rocksdb_column_family_handle_t* column_family,
    char** errptr) noexcept {
  auto* raw =
      new (std::nothrow) rust_rocksdb_table_properties_collection_t();
  if (raw == nullptr) {
    RustSaveStaticError(errptr,
                        "failed to allocate table properties collection");
    return nullptr;
  }

  std::unique_ptr<rust_rocksdb_table_properties_collection_t> collection(raw);
  try {
    Status status = RustDB(db)->GetPropertiesOfAllTables(
        RustColumnFamilyHandle(column_family), &collection->rep);
    if (RustSaveError(errptr, status)) {
      return nullptr;
    }
    return collection.release();
  } catch (...) {
    RustSaveStaticError(errptr, "exception while reading table properties");
    return nullptr;
  }
}

extern "C" void rust_rocksdb_table_properties_collection_destroy(
    rust_rocksdb_table_properties_collection_t* collection) noexcept {
  delete collection;
}

extern "C" size_t rust_rocksdb_table_properties_collection_len(
    const rust_rocksdb_table_properties_collection_t* collection) noexcept {
  return collection->rep.size();
}

extern "C" rust_rocksdb_table_properties_collection_iter_t*
rust_rocksdb_table_properties_collection_iter_create(
    const rust_rocksdb_table_properties_collection_t* collection) noexcept {
  return RustNewOrAbort<rust_rocksdb_table_properties_collection_iter_t>(
      collection->rep.cbegin(), collection->rep.cend());
}

extern "C" void rust_rocksdb_table_properties_collection_iter_destroy(
    rust_rocksdb_table_properties_collection_iter_t* iterator) noexcept {
  delete iterator;
}

extern "C" unsigned char
rust_rocksdb_table_properties_collection_iter_next(
    rust_rocksdb_table_properties_collection_iter_t* iterator,
    const char** file_name, size_t* file_name_len,
    rust_rocksdb_table_properties_t** properties) noexcept {
  if (iterator->current == iterator->end) {
    return 0;
  }

  *file_name = iterator->current->first.data();
  *file_name_len = iterator->current->first.size();
  *properties = RustNewOrAbort<rust_rocksdb_table_properties_t>(
      iterator->current->second);
  ++iterator->current;
  return 1;
}

extern "C" void rust_rocksdb_table_properties_destroy(
    rust_rocksdb_table_properties_t* properties) noexcept {
  delete properties;
}

extern "C" uint64_t rust_rocksdb_table_properties_data_size(
    const rust_rocksdb_table_properties_t* properties) noexcept {
  return properties->rep->data_size;
}

extern "C" uint64_t rust_rocksdb_table_properties_index_size(
    const rust_rocksdb_table_properties_t* properties) noexcept {
  return properties->rep->index_size;
}

extern "C" uint64_t rust_rocksdb_table_properties_filter_size(
    const rust_rocksdb_table_properties_t* properties) noexcept {
  return properties->rep->filter_size;
}

extern "C" uint64_t rust_rocksdb_table_properties_raw_key_size(
    const rust_rocksdb_table_properties_t* properties) noexcept {
  return properties->rep->raw_key_size;
}

extern "C" uint64_t rust_rocksdb_table_properties_raw_value_size(
    const rust_rocksdb_table_properties_t* properties) noexcept {
  return properties->rep->raw_value_size;
}

extern "C" uint64_t rust_rocksdb_table_properties_num_data_blocks(
    const rust_rocksdb_table_properties_t* properties) noexcept {
  return properties->rep->num_data_blocks;
}

extern "C" uint64_t rust_rocksdb_table_properties_num_entries(
    const rust_rocksdb_table_properties_t* properties) noexcept {
  return properties->rep->num_entries;
}

extern "C" uint64_t rust_rocksdb_table_properties_num_deletions(
    const rust_rocksdb_table_properties_t* properties) noexcept {
  return properties->rep->num_deletions;
}

extern "C" uint64_t rust_rocksdb_table_properties_num_merge_operands(
    const rust_rocksdb_table_properties_t* properties) noexcept {
  return properties->rep->num_merge_operands;
}

extern "C" uint64_t rust_rocksdb_table_properties_num_range_deletions(
    const rust_rocksdb_table_properties_t* properties) noexcept {
  return properties->rep->num_range_deletions;
}

static rust_rocksdb_user_collected_properties_iter_t* RustPropertiesIter(
    const UserCollectedProperties& properties) noexcept {
  return RustNewOrAbort<rust_rocksdb_user_collected_properties_iter_t>(
      properties.cbegin(), properties.cend());
}

extern "C" rust_rocksdb_user_collected_properties_iter_t*
rust_rocksdb_table_properties_user_collected_properties_iter_create(
    const rust_rocksdb_table_properties_t* properties) noexcept {
  return RustPropertiesIter(properties->rep->user_collected_properties);
}

extern "C" rust_rocksdb_user_collected_properties_iter_t*
rust_rocksdb_table_properties_readable_properties_iter_create(
    const rust_rocksdb_table_properties_t* properties) noexcept {
  return RustPropertiesIter(properties->rep->readable_properties);
}

extern "C" void rust_rocksdb_user_collected_properties_iter_destroy(
    rust_rocksdb_user_collected_properties_iter_t* iterator) noexcept {
  delete iterator;
}

extern "C" unsigned char
rust_rocksdb_user_collected_properties_iter_next(
    rust_rocksdb_user_collected_properties_iter_t* iterator, const char** key,
    size_t* key_len, const char** value, size_t* value_len) noexcept {
  if (iterator->current == iterator->end) {
    return 0;
  }

  *key = iterator->current->first.data();
  *key_len = iterator->current->first.size();
  *value = iterator->current->second.data();
  *value_len = iterator->current->second.size();
  ++iterator->current;
  return 1;
}
