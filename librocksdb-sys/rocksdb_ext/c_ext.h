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

/* Table Properties Collector Factory */
typedef struct rocksdb_table_properties_collector_t rocksdb_table_properties_collector_t;
typedef struct rocksdb_table_properties_collector_factory_t rocksdb_table_properties_collector_factory_t;
typedef struct rocksdb_entry_type_t rocksdb_entry_type_t;
typedef struct rocksdb_sequence_number_t rocksdb_sequence_number_t;
typedef struct rocksdb_user_collected_properties_t rocksdb_user_collected_properties_t;

typedef struct rocksdb_table_properties_collector_context_t
    rocksdb_table_properties_collector_context_t;

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

#ifdef __cplusplus
}
#endif

