// Copyright (c) Facebook, Inc. and its affiliates. All Rights Reserved.

#pragma once

#ifdef _WIN32
#ifdef ROCKSDB_DLL
#ifdef ROCKSDB_LIBRARY_EXPORTS
#define ROCKSDB_LIBRARY_API __declspec(dllexport)
#else
#define ROCKSDB_LIBRARY_API __declspec(dllimport)
#endif
#else
#define ROCKSDB_LIBRARY_API
#endif
#else
#define ROCKSDB_LIBRARY_API
#endif

#include <rocksdb/c.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct rocksdb_table_properties_collector_t rocksdb_table_properties_collector_t;
typedef struct rocksdb_table_properties_collector_factory_t rocksdb_table_properties_collector_factory_t;
typedef struct rocksdb_entry_type_t rocksdb_entry_type_t;
typedef struct rocksdb_sequence_number_t rocksdb_sequence_number_t;
typedef struct rocksdb_user_collected_properties_t rocksdb_user_collected_properties_t;
typedef struct rocksdb_table_properties_collector_context_t rocksdb_table_properties_collector_context_t;
typedef struct rocksdb_table_properties_t rocksdb_table_properties_t;
typedef struct rocksdb_table_properties_collection_t rocksdb_table_properties_collection_t;
typedef struct rocksdb_table_properties_collection_iter_t rocksdb_table_properties_collection_iter_t;
typedef struct rocksdb_user_collected_properties_iter_t rocksdb_user_collected_properties_iter_t;

struct rocksdb_table_properties_collector_t;
struct rocksdb_table_properties_collector_factory_t;
struct rocksdb_entry_type_t;
struct rocksdb_sequence_number_t;
struct rocksdb_user_collected_properties_t;
struct rocksdb_table_properties_collector_context_t;

struct rocksdb_table_properties_t {
    void* opaque;
};

struct rocksdb_table_properties_collection_t {
    void* opaque;
};

struct rocksdb_table_properties_collection_iter_t {
    void* opaque_iter;
    void* opaque_end;
};

struct rocksdb_user_collected_properties_iter_t {
    void* opaque_iter;
    void* opaque_end;
};

extern ROCKSDB_LIBRARY_API uint32_t
rocksdb_tablepropertiescollectorcontext_column_family_id(
    rocksdb_table_properties_collector_context_t* context);

extern ROCKSDB_LIBRARY_API int
rocksdb_tablepropertiescollectorcontext_level_at_creation(
    rocksdb_table_properties_collector_context_t* context);

extern ROCKSDB_LIBRARY_API int
rocksdb_tablepropertiescollectorcontext_num_levels(
    rocksdb_table_properties_collector_context_t* context);

extern ROCKSDB_LIBRARY_API uint64_t
rocksdb_tablepropertiescollectorcontext_last_level_inclusive_max_seqno_threshold(
    rocksdb_table_properties_collector_context_t* context);


#define ROCKSDB_ENTRY_TYPE_PUT 0
#define ROCKSDB_ENTRY_TYPE_DELETE 1
#define ROCKSDB_ENTRY_TYPE_SINGLE_DELETE 2
#define ROCKSDB_ENTRY_TYPE_MERGE 3
#define ROCKSDB_ENTRY_TYPE_RANGE_DELETION 4
#define ROCKSDB_ENTRY_TYPE_BLOB_INDEX 5
#define ROCKSDB_ENTRY_TYPE_DELETE_WITH_TIMESTAMP 6
#define ROCKSDB_ENTRY_TYPE_WIDE_COLUMN_ENTITY 7
#define ROCKSDB_ENTRY_TYPE_TIMED_PUT 8
#define ROCKSDB_ENTRY_TYPE_OTHER 9

typedef void (*add_cb)(void*, const char* key, size_t key_len, const char* value, size_t value_len, char**);
typedef void (*add_user_key_cb)(void*, const char* key, size_t key_len, const char* value, size_t value_len, rocksdb_entry_type_t* entry_type, rocksdb_sequence_number_t* seq, uint64_t file_size, char**);
typedef void (*block_add_cb)(void*, uint64_t, uint64_t, uint64_t);
typedef void (*finish_cb)(void*, rocksdb_user_collected_properties_t* props, char**);
typedef void (*get_readable_properties_cb)(void*, rocksdb_user_collected_properties_t* props);
typedef const char* (*name_cb)(void*);
typedef bool (*need_compact_cb)(void*);

extern ROCKSDB_LIBRARY_API rocksdb_table_properties_collector_t*
rocksdb_table_properties_collector_create(
    void* state,
    void (*destructor)(void*),
    add_cb add,
    add_user_key_cb add_user_key,
    block_add_cb block_add,
    finish_cb finish,
    get_readable_properties_cb get_readable_properties,
    name_cb name,
    need_compact_cb need_compact);

extern ROCKSDB_LIBRARY_API void
rocksdb_table_properties_collector_destroy(
    rocksdb_table_properties_collector_t* collector);

extern ROCKSDB_LIBRARY_API rocksdb_table_properties_collector_factory_t*
rocksdb_table_properties_collector_factory_create(
    void* state,
    void (*destructor)(void*),
    rocksdb_table_properties_collector_t* (*create_collector)(
        void*, rocksdb_table_properties_collector_context_t* context),
    const char* (*name)(void*));

extern ROCKSDB_LIBRARY_API void
rocksdb_table_properties_collector_factory_destroy(
    rocksdb_table_properties_collector_factory_t* factory);

extern ROCKSDB_LIBRARY_API void
rocksdb_options_add_table_properties_collector_factory(
    rocksdb_options_t* opt,
    rocksdb_table_properties_collector_factory_t* factory);

extern ROCKSDB_LIBRARY_API void
rocksdb_user_collected_properties_add(
    rocksdb_user_collected_properties_t* props,
    const char* key,
    size_t key_len,
    const char* value,
    size_t value_len);

extern ROCKSDB_LIBRARY_API void
rocksdb_user_collected_properties_clear(
    rocksdb_user_collected_properties_t* props);

extern ROCKSDB_LIBRARY_API rocksdb_table_properties_collection_t*
rocksdb_get_properties_of_all_tables(rocksdb_t* db, char** errptr);

extern ROCKSDB_LIBRARY_API rocksdb_table_properties_collection_t*
rocksdb_get_properties_of_all_tables_cf(rocksdb_t* db, rocksdb_column_family_handle_t* column_family, char** errptr);

extern ROCKSDB_LIBRARY_API void
rocksdb_table_properties_collection_destroy(rocksdb_table_properties_collection_t* collection);

extern ROCKSDB_LIBRARY_API size_t
rocksdb_table_properties_collection_len(rocksdb_table_properties_collection_t* collection);

extern ROCKSDB_LIBRARY_API rocksdb_table_properties_collection_iter_t*
rocksdb_table_properties_collection_iter_create(rocksdb_table_properties_collection_t* collection);

extern ROCKSDB_LIBRARY_API void
rocksdb_table_properties_collection_iter_destroy(rocksdb_table_properties_collection_iter_t* iter);

extern ROCKSDB_LIBRARY_API bool
rocksdb_table_properties_collection_iter_next(rocksdb_table_properties_collection_iter_t* iter,
                                              const char** key, size_t* key_len,
                                              rocksdb_table_properties_t** props);

extern ROCKSDB_LIBRARY_API void
rocksdb_table_properties_destroy(rocksdb_table_properties_t* props);

extern ROCKSDB_LIBRARY_API uint64_t rocksdb_table_properties_get_data_size(const rocksdb_table_properties_t* props);
extern ROCKSDB_LIBRARY_API uint64_t rocksdb_table_properties_get_index_size(const rocksdb_table_properties_t* props);
extern ROCKSDB_LIBRARY_API uint64_t rocksdb_table_properties_get_filter_size(const rocksdb_table_properties_t* props);
extern ROCKSDB_LIBRARY_API uint64_t rocksdb_table_properties_get_raw_key_size(const rocksdb_table_properties_t* props);
extern ROCKSDB_LIBRARY_API uint64_t rocksdb_table_properties_get_raw_value_size(const rocksdb_table_properties_t* props);
extern ROCKSDB_LIBRARY_API uint64_t rocksdb_table_properties_get_num_data_blocks(const rocksdb_table_properties_t* props);
extern ROCKSDB_LIBRARY_API uint64_t rocksdb_table_properties_get_num_entries(const rocksdb_table_properties_t* props);
extern ROCKSDB_LIBRARY_API uint64_t rocksdb_table_properties_get_num_deletions(const rocksdb_table_properties_t* props);
extern ROCKSDB_LIBRARY_API uint64_t rocksdb_table_properties_get_num_merge_operands(const rocksdb_table_properties_t* props);
extern ROCKSDB_LIBRARY_API uint64_t rocksdb_table_properties_get_num_range_deletions(const rocksdb_table_properties_t* props);

extern ROCKSDB_LIBRARY_API const rocksdb_user_collected_properties_t*
rocksdb_table_properties_get_user_collected_properties(const rocksdb_table_properties_t* props);

extern ROCKSDB_LIBRARY_API const rocksdb_user_collected_properties_t*
rocksdb_table_properties_get_readable_properties(const rocksdb_table_properties_t* props);

extern ROCKSDB_LIBRARY_API rocksdb_user_collected_properties_iter_t*
rocksdb_user_collected_properties_iter_create(const rocksdb_user_collected_properties_t* props);

extern ROCKSDB_LIBRARY_API void
rocksdb_user_collected_properties_iter_destroy(rocksdb_user_collected_properties_iter_t* iter);

extern ROCKSDB_LIBRARY_API bool
rocksdb_user_collected_properties_iter_next(rocksdb_user_collected_properties_iter_t* iter,
                                            const char** key, size_t* key_len,
                                            const char** val, size_t* val_len);

#ifdef __cplusplus
}
#endif

