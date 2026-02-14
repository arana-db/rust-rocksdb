// Copyright (c) Facebook, Inc. and its affiliates. All Rights Reserved.

#include "c_ext.h"

#include <vector>
#include <string>
#include <cstring>
#include <memory>
#include <cstdlib>

#include "rocksdb/table_properties.h"
#include "rocksdb/options.h"
#include "rocksdb/types.h"
#include "rocksdb/c.h"
#include "rocksdb/db.h"

using ROCKSDB_NAMESPACE::TablePropertiesCollector;
using ROCKSDB_NAMESPACE::TablePropertiesCollectorFactory;
using ROCKSDB_NAMESPACE::UserCollectedProperties;
using ROCKSDB_NAMESPACE::EntryType;
using ROCKSDB_NAMESPACE::Status;
using ROCKSDB_NAMESPACE::SequenceNumber;
using ROCKSDB_NAMESPACE::Slice;
using ROCKSDB_NAMESPACE::Options;

// Define structs with actual C++ types before extern "C" block
struct rocksdb_t {
    ROCKSDB_NAMESPACE::DB* rep;
};

struct rocksdb_column_family_handle_t {
    ROCKSDB_NAMESPACE::ColumnFamilyHandle* rep;
};

struct rocksdb_table_properties_t {
    std::shared_ptr<const ROCKSDB_NAMESPACE::TableProperties> rep;
};

struct rocksdb_table_properties_collection_t {
    ROCKSDB_NAMESPACE::TablePropertiesCollection rep;
};

struct rocksdb_table_properties_collection_iter_t {
    ROCKSDB_NAMESPACE::TablePropertiesCollection::const_iterator iter;
    ROCKSDB_NAMESPACE::TablePropertiesCollection::const_iterator end;
};

struct rocksdb_user_collected_properties_iter_t {
    ROCKSDB_NAMESPACE::UserCollectedProperties::const_iterator iter;
    ROCKSDB_NAMESPACE::UserCollectedProperties::const_iterator end;
};

extern "C" {
struct rocksdb_options_t {
    Options rep;
};

struct rocksdb_entry_type_t {
    EntryType rep;
};

struct rocksdb_sequence_number_t {
    SequenceNumber rep;
};

struct rocksdb_user_collected_properties_t {
    UserCollectedProperties rep;
};

struct rocksdb_table_properties_collector_context_t {
    TablePropertiesCollectorFactory::Context rep;
};

struct rocksdb_table_properties_collector_t : public TablePropertiesCollector {
    void* state_;
    void (*destructor_)(void*);
    void (*add_)(void*, const char*, size_t, const char*, size_t, char**);
    void (*add_user_key_)(void*, const char*, size_t, const char*, size_t,
                          rocksdb_entry_type_t*, rocksdb_sequence_number_t*, uint64_t, char**);
    void (*block_add_)(void*, uint64_t, uint64_t, uint64_t);
    void (*finish_)(void*, rocksdb_user_collected_properties_t*, char**);
    void (*get_readable_properties_)(void*, rocksdb_user_collected_properties_t*);
    const char* (*name_)(void*);
    bool (*need_compact_)(void*);

    UserCollectedProperties collected_properties_;

    rocksdb_table_properties_collector_t() = default;
    rocksdb_table_properties_collector_t(const rocksdb_table_properties_collector_t&) = delete;
    rocksdb_table_properties_collector_t& operator=(const rocksdb_table_properties_collector_t&) = delete;
    rocksdb_table_properties_collector_t(rocksdb_table_properties_collector_t&&) = delete;
    rocksdb_table_properties_collector_t& operator=(rocksdb_table_properties_collector_t&&) = delete;

    ~rocksdb_table_properties_collector_t() override {
        if (destructor_) {
            (*destructor_)(state_);
        }
    }

    Status Add(const Slice& key, const Slice& value) override {
        if (add_) {
            char* err = nullptr;
            (*add_)(state_, key.data(), key.size(), value.data(), value.size(), &err);
            if (err) {
                return Status::InvalidArgument(err);
            }
        }
        return Status::OK();
    }

    Status AddUserKey(const Slice& key, const Slice& value,
                     EntryType type, SequenceNumber seq,
                     uint64_t file_size) override {
        if (add_user_key_) {
            rocksdb_entry_type_t entry_type_wrapper;
            entry_type_wrapper.rep = type;
            rocksdb_sequence_number_t seq_wrapper;
            seq_wrapper.rep = seq;
            char* err = nullptr;
            (*add_user_key_)(state_, key.data(), key.size(), value.data(), value.size(),
                            &entry_type_wrapper, &seq_wrapper, file_size, &err);
            if (err) {
                return Status::InvalidArgument(err);
            }
        } else if (add_) {
            return Add(key, value);
        }
        return Status::OK();
    }

    void BlockAdd(uint64_t block_uncomp_bytes,
                 uint64_t block_compressed_bytes_fast,
                 uint64_t block_compressed_bytes_slow) override {
        if (block_add_) {
            (*block_add_)(state_, block_uncomp_bytes,
                         block_compressed_bytes_fast, block_compressed_bytes_slow);
        }
    }

    Status Finish(UserCollectedProperties* properties) override {
        if (!properties) {
            return Status::InvalidArgument("properties pointer is null");
        }

        if (finish_) {
            rocksdb_user_collected_properties_t props_wrapper;
            props_wrapper.rep.clear();
            char* err = nullptr;
            (*finish_)(state_, &props_wrapper, &err);
            if (err) {
                return Status::InvalidArgument(err);
            }
            
            *properties = props_wrapper.rep;
            collected_properties_ = props_wrapper.rep;
        }
        return Status::OK();
    }

    UserCollectedProperties GetReadableProperties() const override {
        if (get_readable_properties_) {
            rocksdb_user_collected_properties_t props_wrapper;
            props_wrapper.rep.clear();
            (*get_readable_properties_)(state_, &props_wrapper);
            return props_wrapper.rep;
        }
        return collected_properties_;
    }

    const char* Name() const override {
        return (*name_)(state_);
    }

    bool NeedCompact() const override {
        if (need_compact_) {
            return (*need_compact_)(state_);
        }
        return false;
    }
};

rocksdb_table_properties_collector_t*
rocksdb_table_properties_collector_create(
    void* state,
    void (*destructor)(void*),
    add_cb add,
    add_user_key_cb add_user_key,
    block_add_cb block_add,
    finish_cb finish,
    get_readable_properties_cb get_readable_properties,
    name_cb name,
    need_compact_cb need_compact) {
    if (!finish || !name) {
        return nullptr;
    }

    rocksdb_table_properties_collector_t* collector =
        new rocksdb_table_properties_collector_t;
    collector->state_ = state;
    collector->destructor_ = destructor;
    collector->name_ = name;
    collector->finish_ = finish;
    collector->add_ = add;
    collector->add_user_key_ = add_user_key;
    collector->block_add_ = block_add;
    collector->get_readable_properties_ = get_readable_properties;
    collector->need_compact_ = need_compact;

    return collector;
}

void rocksdb_table_properties_collector_destroy(
    rocksdb_table_properties_collector_t* collector) {
    if (collector) {
        delete collector;
    }
}

struct rocksdb_table_properties_collector_factory_t : public TablePropertiesCollectorFactory {
    void* state_;
    void (*destructor_)(void*);
    rocksdb_table_properties_collector_t* (*create_table_properties_collector_)(
        void*, rocksdb_table_properties_collector_context_t*);
    const char* (*name_)(void*);

    ~rocksdb_table_properties_collector_factory_t() override { (*destructor_)(state_); }

    TablePropertiesCollector*  CreateTablePropertiesCollector(
        TablePropertiesCollectorFactory::Context context) override {
        rocksdb_table_properties_collector_context_t ccontext;
        ccontext.rep = context;
        rocksdb_table_properties_collector_t* tpc = 
            (*create_table_properties_collector_)(state_, &ccontext);
        return tpc;
    }

    const char* Name() const override { return (*name_)(state_); }
};

rocksdb_table_properties_collector_factory_t*
rocksdb_table_properties_collector_factory_create(
    void* state,
    void (*destructor)(void*),
    rocksdb_table_properties_collector_t* (*create_table_properties_collector)(
        void*, rocksdb_table_properties_collector_context_t*),
    const char* (*name)(void*)) {
    if (!create_table_properties_collector || !name) {
        return nullptr;
    }

    rocksdb_table_properties_collector_factory_t* factory =
        new rocksdb_table_properties_collector_factory_t;
    factory->state_ = state;
    factory->destructor_ = destructor;
    factory->create_table_properties_collector_ = create_table_properties_collector;
    factory->name_ = name;

    return factory;
}

void rocksdb_table_properties_collector_factory_destroy(
    rocksdb_table_properties_collector_factory_t* factory) {
    if (factory) {
        delete factory;
    }
}

void rocksdb_options_add_table_properties_collector_factory(
    rocksdb_options_t* opt,
    rocksdb_table_properties_collector_factory_t* factory) {
    opt->rep.table_properties_collector_factories.emplace_back(
        std::shared_ptr<ROCKSDB_NAMESPACE::TablePropertiesCollectorFactory>(factory));
}

void rocksdb_user_collected_properties_add(
    rocksdb_user_collected_properties_t* props,
    const char* key,
    size_t key_len,
    const char* value,
    size_t value_len) {
    if (props && key && value) {
        std::string key_str(key, key_len);
        std::string value_str(value, value_len);
        props->rep[key_str] = value_str;
    }
}

void rocksdb_user_collected_properties_clear(
    rocksdb_user_collected_properties_t* props) {
    if (props) {
        props->rep.clear();
    }
}

uint32_t rocksdb_tablepropertiescollectorcontext_column_family_id(
    rocksdb_table_properties_collector_context_t* context) {
    return context->rep.column_family_id;
}

int rocksdb_tablepropertiescollectorcontext_level_at_creation(
    rocksdb_table_properties_collector_context_t* context) {
    return context->rep.level_at_creation;
}

int rocksdb_tablepropertiescollectorcontext_num_levels(
    rocksdb_table_properties_collector_context_t* context) {
    return context->rep.num_levels;
}

uint64_t rocksdb_tablepropertiescollectorcontext_last_level_inclusive_max_seqno_threshold(
    rocksdb_table_properties_collector_context_t* context) {
    return context->rep.last_level_inclusive_max_seqno_threshold;
}

// Helper for error handling, matching SaveError in rocksdb/db/c.cc
static bool SaveError(char** errptr, const ROCKSDB_NAMESPACE::Status& s) {
    if (s.ok()) {
        return false;
    }
    if (errptr != nullptr) {
        if (*errptr != nullptr) {
            free(*errptr);
        }
        *errptr = strdup(s.ToString().c_str());
    }
    return true;
}

rocksdb_table_properties_collection_t* rocksdb_get_properties_of_all_tables(rocksdb_t* db, char** errptr) {
    auto coll = new rocksdb_table_properties_collection_t;
    if (SaveError(errptr, db->rep->GetPropertiesOfAllTables(&coll->rep))) {
        delete coll;
        return nullptr;
    }
    return coll;
}

rocksdb_table_properties_collection_t* rocksdb_get_properties_of_all_tables_cf(rocksdb_t* db, rocksdb_column_family_handle_t* column_family, char** errptr) {
    auto coll = new rocksdb_table_properties_collection_t;
    if (SaveError(errptr, db->rep->GetPropertiesOfAllTables(column_family->rep, &coll->rep))) {
        delete coll;
        return nullptr;
    }
    return coll;
}

void rocksdb_table_properties_collection_destroy(rocksdb_table_properties_collection_t* collection) {
    delete collection;
}

size_t rocksdb_table_properties_collection_len(rocksdb_table_properties_collection_t* collection) {
    return collection->rep.size();
}

rocksdb_table_properties_collection_iter_t* rocksdb_table_properties_collection_iter_create(rocksdb_table_properties_collection_t* collection) {
    auto iter = new rocksdb_table_properties_collection_iter_t;
    iter->iter = collection->rep.begin();
    iter->end = collection->rep.end();
    return iter;
}

void rocksdb_table_properties_collection_iter_destroy(rocksdb_table_properties_collection_iter_t* iter) {
    delete iter;
}

bool rocksdb_table_properties_collection_iter_next(rocksdb_table_properties_collection_iter_t* iter, const char** key, size_t* key_len, rocksdb_table_properties_t** props) {
    if (iter->iter == iter->end) {
        return false;
    }
    *key = iter->iter->first.c_str();
    *key_len = iter->iter->first.size();
    *props = new rocksdb_table_properties_t;
    (*props)->rep = iter->iter->second;
    iter->iter++;
    return true;
}

void rocksdb_table_properties_destroy(rocksdb_table_properties_t* props) {
    delete props;
}

uint64_t rocksdb_table_properties_get_data_size(const rocksdb_table_properties_t* props) {
    return props->rep->data_size;
}

uint64_t rocksdb_table_properties_get_index_size(const rocksdb_table_properties_t* props) {
    return props->rep->index_size;
}

uint64_t rocksdb_table_properties_get_filter_size(const rocksdb_table_properties_t* props) {
    return props->rep->filter_size;
}

uint64_t rocksdb_table_properties_get_raw_key_size(const rocksdb_table_properties_t* props) {
    return props->rep->raw_key_size;
}

uint64_t rocksdb_table_properties_get_raw_value_size(const rocksdb_table_properties_t* props) {
    return props->rep->raw_value_size;
}

uint64_t rocksdb_table_properties_get_num_data_blocks(const rocksdb_table_properties_t* props) {
    return props->rep->num_data_blocks;
}

uint64_t rocksdb_table_properties_get_num_entries(const rocksdb_table_properties_t* props) {
    return props->rep->num_entries;
}

uint64_t rocksdb_table_properties_get_num_deletions(const rocksdb_table_properties_t* props) {
    return props->rep->num_deletions;
}

uint64_t rocksdb_table_properties_get_num_merge_operands(const rocksdb_table_properties_t* props) {
    return props->rep->num_merge_operands;
}

uint64_t rocksdb_table_properties_get_num_range_deletions(const rocksdb_table_properties_t* props) {
    return props->rep->num_range_deletions;
}

const rocksdb_user_collected_properties_t*
rocksdb_table_properties_get_user_collected_properties(const rocksdb_table_properties_t* props) {
    // We cannot just cast because rocksdb_user_collected_properties_t might have a different layout
    // Actually rocksdb_user_collected_properties_t is defined as { UserCollectedProperties rep; }
    // So we can return a pointer to the member if we are careful about lifetimes.
    // However, to be safe and match the style of other bindings, we return an opaque pointer.
    return reinterpret_cast<const rocksdb_user_collected_properties_t*>(&props->rep->user_collected_properties);
}

const rocksdb_user_collected_properties_t*
rocksdb_table_properties_get_readable_properties(const rocksdb_table_properties_t* props) {
    return reinterpret_cast<const rocksdb_user_collected_properties_t*>(&props->rep->readable_properties);
}

rocksdb_user_collected_properties_iter_t*
rocksdb_user_collected_properties_iter_create(const rocksdb_user_collected_properties_t* props) {
    auto iter = new rocksdb_user_collected_properties_iter_t;
    // Safety: we know rocksdb_user_collected_properties_t wraps UserCollectedProperties
    const auto& map = *reinterpret_cast<const ROCKSDB_NAMESPACE::UserCollectedProperties*>(props);
    iter->iter = map.begin();
    iter->end = map.end();
    return iter;
}

void rocksdb_user_collected_properties_iter_destroy(rocksdb_user_collected_properties_iter_t* iter) {
    delete iter;
}

bool rocksdb_user_collected_properties_iter_next(rocksdb_user_collected_properties_iter_t* iter, const char** key, size_t* key_len, const char** val, size_t* val_len) {
    if (iter->iter == iter->end) {
        return false;
    }
    *key = iter->iter->first.data();
    *key_len = iter->iter->first.size();
    *val = iter->iter->second.data();
    *val_len = iter->iter->second.size();
    iter->iter++;
    return true;
}

}  // extern "C"
