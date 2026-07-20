# Kiwi rust-rocksdb 维护基线与 TableProperties 只读 API 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 在 `zaidoon1/rust-rocksdb@a27cb5bd` 基线上建立可审计的 Arana Kiwi 维护线，并恢复 Kiwi 读取 SST TableProperties 所需的安全 Rust/C++ API，不引入 Collector callback。

**架构：** 新增的 C++ wrapper 放入现有 `librocksdb-sys/c-api-extensions/`，通过当前 bundled/system backend 和 bindgen 框架编译。Rust 侧用 `NonNull` 和明确的 Drop 责任封装 collection、iterator 和单表 properties；借用 iterator 由生命周期绑定 collection，owning iterator 直接持有 collection，不使用 `std::mem::zeroed()` 或 `forget()`。

**技术栈：** Rust 1.91、rust-librocksdb-sys、RocksDB 11.1.2 C++ API、bindgen、Cargo integration tests、PowerShell、WSL/Linux。

---

## 范围边界

本计划只交付：

- 可审计的 Kiwi maintenance baseline 文档。
- TableProperties collection 和单表只读数据模型。
- `DB::get_properties_of_all_tables()`。
- `DB::get_properties_of_all_tables_cf()`。
- numeric properties、user-collected properties 和 readable properties。
- 借用 iterator 和 owning iterator 的安全所有权。
- focused tests、fmt、clippy、doctest 和 bundled build 验证。

本计划不交付：

- `TablePropertiesCollector` callback。
- `TablePropertiesCollectorFactory`。
- `Options::set_table_properties_collector_factory`。
- callback panic containment。
- Factory 并发测试。
- Kiwi 仓库 revision 更新。

这些能力依赖本计划确定的 C handle 和 Rust 所有权模型，必须在本计划验证后用独立计划实施。

## 文件结构

- 创建：`docs/kiwi-maintenance-baseline.md`
  - 面向维护者记录 upstream SHA、submodule、MSRV、远端边界和旧 Arana 来源。
- 修改：`librocksdb-sys/c-api-extensions/c_api_extensions.h`
  - 声明带 `rust_rocksdb_` 前缀的 TableProperties 只读 C API。
- 修改：`librocksdb-sys/c-api-extensions/c_api_extensions.cc`
  - 封装 RocksDB `DB::GetPropertiesOfAllTables`、collection iterator 和 properties getters。
- 创建：`src/table_properties.rs`
  - Rust 所有权封装、Drop、借用 iterator、owning iterator 和 property map 拷贝。
- 修改：`src/db.rs`
  - 在 `DBCommon<T, D>` 中暴露默认 CF 和指定 CF 的读取方法。
- 修改：`src/lib.rs`
  - 声明模块并 re-export 公共类型。
- 创建：`tests/test_table_properties_read.rs`
  - 默认 CF、指定 CF、空 DB、numeric getters、property maps 和 iterator 生命周期回归。

## API 合同

Rust 公共接口固定为：

```rust
pub struct TablePropertiesCollection {
    inner: NonNull<ffi::rust_rocksdb_table_properties_collection_t>,
}

pub struct TablePropertiesCollectionIter<'a> {
    inner: NonNull<ffi::rust_rocksdb_table_properties_collection_iter_t>,
    _collection: PhantomData<&'a TablePropertiesCollection>,
}

pub struct TablePropertiesCollectionIntoIter {
    inner: NonNull<ffi::rust_rocksdb_table_properties_collection_iter_t>,
    _collection: TablePropertiesCollection,
}

pub struct TableProperties {
    inner: NonNull<ffi::rust_rocksdb_table_properties_t>,
}

impl TablePropertiesCollection {
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn iter(&self) -> TablePropertiesCollectionIter<'_>;
}

impl<'a> Iterator for TablePropertiesCollectionIter<'a> {
    type Item = (Box<[u8]>, TableProperties);
}

impl Iterator for TablePropertiesCollectionIntoIter {
    type Item = (Box<[u8]>, TableProperties);
}

impl IntoIterator for TablePropertiesCollection {
    type Item = (Box<[u8]>, TableProperties);
    type IntoIter = TablePropertiesCollectionIntoIter;
}

impl TableProperties {
    pub fn data_size(&self) -> u64;
    pub fn index_size(&self) -> u64;
    pub fn filter_size(&self) -> u64;
    pub fn raw_key_size(&self) -> u64;
    pub fn raw_value_size(&self) -> u64;
    pub fn num_data_blocks(&self) -> u64;
    pub fn num_entries(&self) -> u64;
    pub fn num_deletions(&self) -> u64;
    pub fn num_merge_operands(&self) -> u64;
    pub fn num_range_deletions(&self) -> u64;
    pub fn user_collected_properties(&self) -> HashMap<Vec<u8>, Vec<u8>>;
    pub fn readable_properties(&self) -> HashMap<Vec<u8>, Vec<u8>>;
}
```

Table 文件名返回 `Box<[u8]>`，不做有损 UTF-8 转换。Kiwi 当前迭代代码使用 `_` 忽略文件名，因此该类型满足 Kiwi 最小需求。

本阶段不为这些 raw C++ handle 添加 `unsafe impl Send/Sync`。只有后续取得 RocksDB 生命周期和线程安全证据并增加对应测试后才能扩展自动 trait。

---

### 任务 1：固定隔离环境和运行 upstream 基线门禁

**文件：**
- 不修改源码。
- 验证：`Cargo.toml`
- 验证：`librocksdb-sys/rocksdb`
- 验证：`librocksdb-sys/snappy`

- [ ] **步骤 1：确认当前分支和精确 upstream 父提交**

运行：

```powershell
git branch --show-current
git rev-parse HEAD
git rev-parse actual-upstream/master
git remote -v
git status --porcelain=v1
```

预期：

```text
codex/kiwi-maintenance-rebuild
HEAD 为设计/计划文档提交，历史中包含 a27cb5bdbdb74550835ed5820ad02817c9a8c457
actual-upstream/master 为 a27cb5bdbdb74550835ed5820ad02817c9a8c457
actual-upstream push URL 为 DISABLED
工作树无输出
```

- [ ] **步骤 2：验证 upstream 是当前分支祖先**

运行：

```powershell
git merge-base --is-ancestor a27cb5bdbdb74550835ed5820ad02817c9a8c457 HEAD
if ($LASTEXITCODE -ne 0) { throw 'upstream base is not an ancestor' }
```

预期：退出码 `0`。

- [ ] **步骤 3：使用现有 gh 凭据初始化 submodule，不在命令或日志中打印 token**

运行：

```powershell
$token = gh auth token
$raw = [System.Text.Encoding]::ASCII.GetBytes("x-access-token:$token")
$header = 'Authorization: Basic ' + [Convert]::ToBase64String($raw)

git -c "http.extraHeader=$header" submodule update --init --recursive
$code = $LASTEXITCODE

Remove-Variable token, raw, header
if ($code -ne 0) { exit $code }
```

预期：RocksDB 和 Snappy submodule 初始化成功，日志不包含明文 token。

- [ ] **步骤 4：核对 submodule SHA**

运行：

```powershell
git submodule status --recursive
git -C librocksdb-sys/rocksdb rev-parse HEAD
git -C librocksdb-sys/snappy rev-parse HEAD
```

预期：

```text
RocksDB: 3b446089141659fad25328c5ea3e7ed283df46e4
Snappy:  6af9287fbdb913f0794d0148c6aa43b58e63c8e3
submodule status 行首没有 +、- 或 U
```

- [ ] **步骤 5：运行格式和 focused baseline build**

运行：

```powershell
cargo fmt --all -- --check
cargo test --test test_property --features multi-threaded-cf
```

预期：两条命令退出码 `0`。如果失败，保存完整日志并停止，不把基线失败归到后续改动。

- [ ] **步骤 6：在 WSL/Linux 复核 focused baseline**

运行：

```bash
cd /mnt/d/test/github/review/rust-rocksdb-maintenance
cargo fmt --all -- --check
cargo test --test test_property --features multi-threaded-cf
```

预期：两条命令退出码 `0`。若 Linux 工具链或依赖缺失，记录具体错误并先修复环境。

---

### 任务 2：新增用户可读的维护基线文档

**文件：**
- 创建：`docs/kiwi-maintenance-baseline.md`
- 参考：`docs/superpowers/specs/2026-07-20-rust-rocksdb-kiwi-maintenance-design.md`

- [ ] **步骤 1：创建维护基线文档**

写入完整内容：

```markdown
# Kiwi rust-rocksdb Maintenance Baseline

This branch is the Arana-owned maintenance line consumed by Kiwi.

## Source baseline

- Source repository: `https://github.com/zaidoon1/rust-rocksdb.git`
- Source commit: `a27cb5bdbdb74550835ed5820ad02817c9a8c457`
- rust-rocksdb: `0.51.0`
- rust-librocksdb-sys: `0.47.1+11.1.2`
- MSRV: Rust `1.91`
- RocksDB submodule: `3b446089141659fad25328c5ea3e7ed283df46e4`
- Snappy submodule: `6af9287fbdb913f0794d0148c6aa43b58e63c8e3`

The local Git remote name is `actual-upstream`. Its push URL must remain
`DISABLED`. This project consumes public source from that repository but does
not push branches, tags, pull requests, issues, discussions, or comments to it.

## Arana compatibility source

- Previous direct baseline: `4f973bf6d94d8a3b32a39697b63092de08106974`
- Previous Kiwi pin: `f7abb18c64fac810f3c4736aef833c340396449b`

The previous implementation is a behavioral reference only. Its commits are
not cherry-picked because the build system, local C API extension framework,
RocksDB version, callback safety requirements, and ownership model changed.

## Maintenance policy

1. `kiwi-maintenance` is the base branch for Arana-specific changes.
2. Upstream synchronization is performed before Arana patches are replayed.
3. Each Arana capability is introduced in a focused, tested commit.
4. RocksDB submodule sources are not modified by Arana extensions.
5. Kiwi pins a reviewed Arana commit SHA; it does not pin the external source.
```

- [ ] **步骤 2：检查文档没有占位符或错误 SHA**

运行：

```powershell
rg -n -i 'T[B]D|待[定]' docs/kiwi-maintenance-baseline.md
rg -n 'a27cb5bdbdb74550835ed5820ad02817c9a8c457|3b446089141659fad25328c5ea3e7ed283df46e4|6af9287fbdb913f0794d0148c6aa43b58e63c8e3|f7abb18c64fac810f3c4736aef833c340396449b' docs/kiwi-maintenance-baseline.md
git diff --check
```

预期：占位符扫描无匹配；四个 SHA 均有匹配；`git diff --check` 退出码 `0`。

- [ ] **步骤 3：提交维护基线文档**

运行：

```powershell
git add docs/kiwi-maintenance-baseline.md
git diff --cached --check
git diff --cached --name-status
git commit -m "chore: establish Kiwi maintenance baseline"
```

预期：暂存区只包含 `docs/kiwi-maintenance-baseline.md`，提交成功。

---

### 任务 3：先编写 TableProperties 公共 API 失败测试

**文件：**
- 创建：`tests/test_table_properties_read.rs`
- 参考：`tests/util/mod.rs`
- 参考：`tests/test_property.rs`

- [ ] **步骤 1：创建失败测试文件**

写入：

```rust
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

use std::collections::HashMap;

use rust_rocksdb::{DB, Options, TableProperties, TablePropertiesCollection};
use util::DBPath;

fn open_db(name: &str) -> (DBPath, DB) {
    let path = DBPath::new(name);
    let mut options = Options::default();
    options.create_if_missing(true);
    let db = DB::open(&options, &path).expect("open test database");
    (path, db)
}

fn assert_numeric_properties(properties: &TableProperties) {
    let _ = properties.data_size();
    let _ = properties.index_size();
    let _ = properties.filter_size();
    let _ = properties.raw_key_size();
    let _ = properties.raw_value_size();
    let _ = properties.num_data_blocks();
    let _ = properties.num_entries();
    let _ = properties.num_deletions();
    let _ = properties.num_merge_operands();
    let _ = properties.num_range_deletions();
}

#[test]
fn empty_database_returns_empty_collection() {
    let (_path, db) = open_db("_rust_rocksdb_table_properties_empty");
    let collection: TablePropertiesCollection = db
        .get_properties_of_all_tables()
        .expect("read table properties from empty database");

    assert_eq!(collection.len(), 0);
    assert!(collection.is_empty());
    assert_eq!(collection.iter().count(), 0);
}

#[test]
fn reads_default_column_family_properties() {
    let (_path, db) = open_db("_rust_rocksdb_table_properties_default_cf");
    db.put(b"key-1", b"value-1").expect("put key-1");
    db.put(b"key-2", b"value-2").expect("put key-2");
    db.flush().expect("flush default column family");

    let collection = db
        .get_properties_of_all_tables()
        .expect("read default column family table properties");

    assert!(!collection.is_empty());
    assert_eq!(collection.len(), collection.iter().count());

    for (file_name, properties) in collection.iter() {
        assert!(!file_name.is_empty());
        assert!(properties.num_entries() >= 1);
        assert_numeric_properties(&properties);

        let user: HashMap<Vec<u8>, Vec<u8>> = properties.user_collected_properties();
        let readable: HashMap<Vec<u8>, Vec<u8>> = properties.readable_properties();
        let _ = (user.len(), readable.len());
    }
}

#[test]
fn reads_named_column_family_properties() {
    let path = DBPath::new("_rust_rocksdb_table_properties_named_cf");
    let mut options = Options::default();
    options.create_if_missing(true);
    options.create_missing_column_families(true);

    let db = DB::open_cf(&options, &path, ["cf1"]).expect("open database with cf1");
    let cf = db.cf_handle("cf1").expect("get cf1 handle");
    db.put_cf(&cf, b"cf-key", b"cf-value")
        .expect("put cf1 value");
    db.flush_cf(&cf).expect("flush cf1");

    let collection = db
        .get_properties_of_all_tables_cf(&cf)
        .expect("read cf1 table properties");

    assert!(!collection.is_empty());
    assert!(collection.iter().any(|(_, properties)| properties.num_entries() >= 1));
}

#[test]
fn borrowed_iterator_can_be_dropped_without_invalidating_collection() {
    let (_path, db) = open_db("_rust_rocksdb_table_properties_borrowed_iter");
    db.put(b"key", b"value").expect("put value");
    db.flush().expect("flush database");

    let collection = db
        .get_properties_of_all_tables()
        .expect("read table properties");

    let mut iterator = collection.iter();
    assert!(iterator.next().is_some());
    drop(iterator);

    assert_eq!(collection.iter().count(), collection.len());
}

#[test]
fn owning_iterator_keeps_collection_alive_until_drop() {
    let (_path, db) = open_db("_rust_rocksdb_table_properties_owning_iter");
    db.put(b"key", b"value").expect("put value");
    db.flush().expect("flush database");

    let collection = db
        .get_properties_of_all_tables()
        .expect("read table properties");
    let expected_len = collection.len();
    let mut iterator = collection.into_iter();

    assert!(iterator.next().is_some());
    let consumed = 1 + iterator.count();
    assert_eq!(consumed, expected_len);
}

#[test]
fn partially_consumed_owning_iterator_can_be_dropped() {
    let (_path, db) = open_db("_rust_rocksdb_table_properties_partial_into_iter");
    db.put(b"key", b"value").expect("put value");
    db.flush().expect("flush database");

    let collection = db
        .get_properties_of_all_tables()
        .expect("read table properties");
    let mut iterator = collection.into_iter();
    let _ = iterator.next();
    drop(iterator);
}
```

- [ ] **步骤 2：运行测试并确认因公共 API 缺失而失败**

运行：

```powershell
cargo test --test test_table_properties_read --features multi-threaded-cf --no-run
```

预期：FAIL，错误包含以下至少一项：

```text
unresolved imports rust_rocksdb::TableProperties
unresolved imports rust_rocksdb::TablePropertiesCollection
no method named get_properties_of_all_tables
```

若测试因为许可证、语法或现有 API 使用错误而失败，先修正测试，直到失败原因只剩待实现 API。

---

### 任务 4：扩展当前 C API extension 的只读 TableProperties 接口

**文件：**
- 修改：`librocksdb-sys/c-api-extensions/c_api_extensions.h`
- 修改：`librocksdb-sys/c-api-extensions/c_api_extensions.cc`
- 参考：`librocksdb-sys/build.rs:230-267`
- 参考：`librocksdb-sys/build.rs:1441-1480`

- [ ] **步骤 1：在 extension header 声明 opaque handles**

在 `extern "C"` 声明区域增加：

```c
typedef struct rust_rocksdb_table_properties_collection_t
    rust_rocksdb_table_properties_collection_t;
typedef struct rust_rocksdb_table_properties_collection_iter_t
    rust_rocksdb_table_properties_collection_iter_t;
typedef struct rust_rocksdb_table_properties_t
    rust_rocksdb_table_properties_t;
typedef struct rust_rocksdb_user_collected_properties_iter_t
    rust_rocksdb_user_collected_properties_iter_t;

#ifdef __cplusplus
#define RUST_ROCKSDB_NOEXCEPT noexcept
#else
#define RUST_ROCKSDB_NOEXCEPT
#endif
```

以下所有新增声明和对应 C++ 定义都必须使用同一异常规范；不得只在 `.cc`
定义上增加 `noexcept`，否则 C++ 编译单元看到的声明与定义不一致。header 的
新增声明区结束后执行 `#undef RUST_ROCKSDB_NOEXCEPT`。

- [ ] **步骤 2：声明 DB 和 collection API**

增加完整声明：

```c
extern ROCKSDB_LIBRARY_API rust_rocksdb_table_properties_collection_t*
rust_rocksdb_get_properties_of_all_tables(rocksdb_t*, char**)
    RUST_ROCKSDB_NOEXCEPT;

extern ROCKSDB_LIBRARY_API rust_rocksdb_table_properties_collection_t*
rust_rocksdb_get_properties_of_all_tables_cf(
    rocksdb_t*, rocksdb_column_family_handle_t*, char**)
    RUST_ROCKSDB_NOEXCEPT;

extern ROCKSDB_LIBRARY_API void
rust_rocksdb_table_properties_collection_destroy(
    rust_rocksdb_table_properties_collection_t*) RUST_ROCKSDB_NOEXCEPT;

extern ROCKSDB_LIBRARY_API size_t
rust_rocksdb_table_properties_collection_len(
    const rust_rocksdb_table_properties_collection_t*) RUST_ROCKSDB_NOEXCEPT;

extern ROCKSDB_LIBRARY_API rust_rocksdb_table_properties_collection_iter_t*
rust_rocksdb_table_properties_collection_iter_create(
    const rust_rocksdb_table_properties_collection_t*) RUST_ROCKSDB_NOEXCEPT;

extern ROCKSDB_LIBRARY_API void
rust_rocksdb_table_properties_collection_iter_destroy(
    rust_rocksdb_table_properties_collection_iter_t*) RUST_ROCKSDB_NOEXCEPT;

extern ROCKSDB_LIBRARY_API unsigned char
rust_rocksdb_table_properties_collection_iter_next(
    rust_rocksdb_table_properties_collection_iter_t*,
    const char**, size_t*, rust_rocksdb_table_properties_t**)
    RUST_ROCKSDB_NOEXCEPT;
```

- [ ] **步骤 3：声明单表 numeric 和 map iterator API**

增加：

```c
extern ROCKSDB_LIBRARY_API void rust_rocksdb_table_properties_destroy(
    rust_rocksdb_table_properties_t*);
extern ROCKSDB_LIBRARY_API uint64_t rust_rocksdb_table_properties_data_size(
    const rust_rocksdb_table_properties_t*);
extern ROCKSDB_LIBRARY_API uint64_t rust_rocksdb_table_properties_index_size(
    const rust_rocksdb_table_properties_t*);
extern ROCKSDB_LIBRARY_API uint64_t rust_rocksdb_table_properties_filter_size(
    const rust_rocksdb_table_properties_t*);
extern ROCKSDB_LIBRARY_API uint64_t rust_rocksdb_table_properties_raw_key_size(
    const rust_rocksdb_table_properties_t*);
extern ROCKSDB_LIBRARY_API uint64_t rust_rocksdb_table_properties_raw_value_size(
    const rust_rocksdb_table_properties_t*);
extern ROCKSDB_LIBRARY_API uint64_t rust_rocksdb_table_properties_num_data_blocks(
    const rust_rocksdb_table_properties_t*);
extern ROCKSDB_LIBRARY_API uint64_t rust_rocksdb_table_properties_num_entries(
    const rust_rocksdb_table_properties_t*);
extern ROCKSDB_LIBRARY_API uint64_t rust_rocksdb_table_properties_num_deletions(
    const rust_rocksdb_table_properties_t*);
extern ROCKSDB_LIBRARY_API uint64_t rust_rocksdb_table_properties_num_merge_operands(
    const rust_rocksdb_table_properties_t*);
extern ROCKSDB_LIBRARY_API uint64_t rust_rocksdb_table_properties_num_range_deletions(
    const rust_rocksdb_table_properties_t*);

extern ROCKSDB_LIBRARY_API rust_rocksdb_user_collected_properties_iter_t*
rust_rocksdb_table_properties_user_collected_properties_iter_create(
    const rust_rocksdb_table_properties_t*);
extern ROCKSDB_LIBRARY_API rust_rocksdb_user_collected_properties_iter_t*
rust_rocksdb_table_properties_readable_properties_iter_create(
    const rust_rocksdb_table_properties_t*);
extern ROCKSDB_LIBRARY_API void
rust_rocksdb_user_collected_properties_iter_destroy(
    rust_rocksdb_user_collected_properties_iter_t*);
extern ROCKSDB_LIBRARY_API unsigned char
rust_rocksdb_user_collected_properties_iter_next(
    rust_rocksdb_user_collected_properties_iter_t*,
    const char**, size_t*, const char**, size_t*);
```

上述 numeric、map iterator create/destroy/next 声明也全部追加
`RUST_ROCKSDB_NOEXCEPT`；对应 `.cc` 定义全部显式标记 `noexcept`。

- [ ] **步骤 4：在 C++ 文件增加 includes 和 type aliases**

增加：

```cpp
#include <cstdlib>
#include <new>
#include <unordered_map>
#include <utility>

#include "rocksdb/db.h"
#include "rocksdb/table_properties.h"

using ROCKSDB_NAMESPACE::ColumnFamilyHandle;
using ROCKSDB_NAMESPACE::TableProperties;
using ROCKSDB_NAMESPACE::TablePropertiesCollection;
using ROCKSDB_NAMESPACE::UserCollectedProperties;
```

- [ ] **步骤 5：定义 extension 自有 handle**

增加：

```cpp
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

static DB* RustDB(rocksdb_t* db) {
  return *reinterpret_cast<DB**>(db);
}

static ColumnFamilyHandle* RustColumnFamilyHandle(
    rocksdb_column_family_handle_t* column_family) {
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
```

在相邻注释中记录布局证据：RocksDB `db/c.cc` 的 `rocksdb_t` 第一字段为 `DB* rep`，`rocksdb_column_family_handle_t` 第一字段为 `ColumnFamilyHandle* rep`。该假设必须由集成测试覆盖。

`RustNewOrAbort` 是这个兼容层的明确 OOM 策略：与 Rust 默认分配器一致，
不可恢复的 handle 分配失败会终止进程；不得让 C++ 异常跨越 C ABI，也不得把
分配失败伪装为迭代结束。能够通过现有 `Result` 返回的 DB 查询入口则必须捕获
所有 C++ 异常并返回错误。

- [ ] **步骤 6：实现默认 CF 和指定 CF 查询**

实现：

```cpp
extern "C" rust_rocksdb_table_properties_collection_t*
rust_rocksdb_get_properties_of_all_tables(rocksdb_t* db,
                                           char** errptr) noexcept {
  auto* raw = new (std::nothrow) rust_rocksdb_table_properties_collection_t();
  if (raw == nullptr) {
    RustSaveStaticError(errptr, "failed to allocate table properties collection");
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
  auto* raw = new (std::nothrow) rust_rocksdb_table_properties_collection_t();
  if (raw == nullptr) {
    RustSaveStaticError(errptr, "failed to allocate table properties collection");
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
```

- [ ] **步骤 7：实现 collection 和 iterator**

实现：

```cpp
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
```

- [ ] **步骤 8：实现 numeric getters**

每个 getter 直接读取 `properties->rep` 的只读字段。例如：

以下所有 getter 的定义都必须显式标记 `noexcept`，并与 header 中的
`RUST_ROCKSDB_NOEXCEPT` 声明保持一致。

```cpp
extern "C" void rust_rocksdb_table_properties_destroy(
    rust_rocksdb_table_properties_t* properties) noexcept {
  delete properties;
}

extern "C" uint64_t rust_rocksdb_table_properties_data_size(
    const rust_rocksdb_table_properties_t* properties) {
  return properties->rep->data_size;
}

extern "C" uint64_t rust_rocksdb_table_properties_num_entries(
    const rust_rocksdb_table_properties_t* properties) {
  return properties->rep->num_entries;
}

extern "C" uint64_t rust_rocksdb_table_properties_index_size(
    const rust_rocksdb_table_properties_t* properties) {
  return properties->rep->index_size;
}

extern "C" uint64_t rust_rocksdb_table_properties_filter_size(
    const rust_rocksdb_table_properties_t* properties) {
  return properties->rep->filter_size;
}

extern "C" uint64_t rust_rocksdb_table_properties_raw_key_size(
    const rust_rocksdb_table_properties_t* properties) {
  return properties->rep->raw_key_size;
}

extern "C" uint64_t rust_rocksdb_table_properties_raw_value_size(
    const rust_rocksdb_table_properties_t* properties) {
  return properties->rep->raw_value_size;
}

extern "C" uint64_t rust_rocksdb_table_properties_num_data_blocks(
    const rust_rocksdb_table_properties_t* properties) {
  return properties->rep->num_data_blocks;
}

extern "C" uint64_t rust_rocksdb_table_properties_num_deletions(
    const rust_rocksdb_table_properties_t* properties) {
  return properties->rep->num_deletions;
}

extern "C" uint64_t rust_rocksdb_table_properties_num_merge_operands(
    const rust_rocksdb_table_properties_t* properties) {
  return properties->rep->num_merge_operands;
}

extern "C" uint64_t rust_rocksdb_table_properties_num_range_deletions(
    const rust_rocksdb_table_properties_t* properties) {
  return properties->rep->num_range_deletions;
}
```

- [ ] **步骤 9：实现 properties map iterators**

增加共享 helper 和两个入口：

```cpp
static rust_rocksdb_user_collected_properties_iter_t*
RustPropertiesIter(const UserCollectedProperties& properties) noexcept {
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
    rust_rocksdb_user_collected_properties_iter_t* iterator,
    const char** key, size_t* key_len, const char** value,
    size_t* value_len) noexcept {
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
```

- [ ] **步骤 10：运行 C++/bindgen 编译并确认 Rust 公共测试仍因 Rust wrapper 缺失而失败**

运行：

```powershell
cargo test --package rust-librocksdb-sys --no-run
cargo test --test test_table_properties_read --features multi-threaded-cf --no-run
```

预期：`rust-librocksdb-sys` 编译成功；公共测试仍因 `TableProperties` Rust 类型和 DB 方法缺失而失败，不得出现 extension C++ 编译或 bindgen symbol 缺失错误。

---

### 任务 5：实现 Rust TableProperties 所有权模型

**文件：**
- 创建：`src/table_properties.rs`
- 修改：`src/lib.rs:92-131`

- [ ] **步骤 1：创建模块、imports 和 collection handle**

`src/table_properties.rs` 使用项目 Apache 2.0 header，并增加：

```rust
use std::{
    collections::HashMap,
    marker::PhantomData,
    ptr::NonNull,
    slice,
};

use crate::{Error, ffi};

pub struct TablePropertiesCollection {
    inner: NonNull<ffi::rust_rocksdb_table_properties_collection_t>,
}

impl TablePropertiesCollection {
    pub(crate) unsafe fn from_raw(
        inner: *mut ffi::rust_rocksdb_table_properties_collection_t,
    ) -> Result<Self, Error> {
        NonNull::new(inner)
            .map(|inner| Self { inner })
            .ok_or_else(|| {
                Error::new("RocksDB returned a null table properties collection".to_owned())
            })
    }

    pub fn len(&self) -> usize {
        unsafe { ffi::rust_rocksdb_table_properties_collection_len(self.inner.as_ptr()) }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn iter(&self) -> TablePropertiesCollectionIter<'_> {
        let inner = unsafe {
            ffi::rust_rocksdb_table_properties_collection_iter_create(self.inner.as_ptr())
        };
        TablePropertiesCollectionIter {
            inner: NonNull::new(inner)
                .expect("RocksDB returned a null table properties iterator"),
            _collection: PhantomData,
        }
    }
}

impl Drop for TablePropertiesCollection {
    fn drop(&mut self) {
        unsafe {
            ffi::rust_rocksdb_table_properties_collection_destroy(self.inner.as_ptr());
        }
    }
}
```

不添加 `unsafe impl Send` 或 `unsafe impl Sync`。

- [ ] **步骤 2：实现共享 iterator next helper**

增加：

```rust
unsafe fn copy_bytes(ptr: *const libc::c_char, len: usize) -> Vec<u8> {
    if len == 0 {
        return Vec::new();
    }
    assert!(!ptr.is_null(), "RocksDB returned a null pointer with non-zero length");
    unsafe { slice::from_raw_parts(ptr.cast::<u8>(), len) }.to_vec()
}

unsafe fn next_table_properties(
    inner: NonNull<ffi::rust_rocksdb_table_properties_collection_iter_t>,
) -> Option<(Box<[u8]>, TableProperties)> {
    let mut file_name = std::ptr::null();
    let mut file_name_len = 0;
    let mut properties = std::ptr::null_mut();

    let has_next = unsafe {
        ffi::rust_rocksdb_table_properties_collection_iter_next(
            inner.as_ptr(),
            &raw mut file_name,
            &raw mut file_name_len,
            &raw mut properties,
        )
    };

    if has_next == 0 {
        return None;
    }

    let file_name = unsafe { copy_bytes(file_name, file_name_len) }.into_boxed_slice();
    let properties = unsafe { TableProperties::from_raw(properties) };
    Some((file_name, properties))
}
```

- [ ] **步骤 3：实现借用 iterator**

增加：

```rust
pub struct TablePropertiesCollectionIter<'a> {
    inner: NonNull<ffi::rust_rocksdb_table_properties_collection_iter_t>,
    _collection: PhantomData<&'a TablePropertiesCollection>,
}

impl Iterator for TablePropertiesCollectionIter<'_> {
    type Item = (Box<[u8]>, TableProperties);

    fn next(&mut self) -> Option<Self::Item> {
        unsafe { next_table_properties(self.inner) }
    }
}

impl Drop for TablePropertiesCollectionIter<'_> {
    fn drop(&mut self) {
        unsafe {
            ffi::rust_rocksdb_table_properties_collection_iter_destroy(
                self.inner.as_ptr(),
            );
        }
    }
}
```

- [ ] **步骤 4：实现 owning iterator，不使用 zeroed/forget**

增加：

```rust
pub struct TablePropertiesCollectionIntoIter {
    inner: NonNull<ffi::rust_rocksdb_table_properties_collection_iter_t>,
    _collection: TablePropertiesCollection,
}

impl Iterator for TablePropertiesCollectionIntoIter {
    type Item = (Box<[u8]>, TableProperties);

    fn next(&mut self) -> Option<Self::Item> {
        unsafe { next_table_properties(self.inner) }
    }
}

impl Drop for TablePropertiesCollectionIntoIter {
    fn drop(&mut self) {
        unsafe {
            ffi::rust_rocksdb_table_properties_collection_iter_destroy(
                self.inner.as_ptr(),
            );
        }
    }
}

impl IntoIterator for TablePropertiesCollection {
    type Item = (Box<[u8]>, TableProperties);
    type IntoIter = TablePropertiesCollectionIntoIter;

    fn into_iter(self) -> Self::IntoIter {
        let inner = unsafe {
            ffi::rust_rocksdb_table_properties_collection_iter_create(self.inner.as_ptr())
        };
        TablePropertiesCollectionIntoIter {
            inner: NonNull::new(inner)
                .expect("RocksDB returned a null table properties iterator"),
            _collection: self,
        }
    }
}
```

保留 `_collection` 字段，即使只用于所有权，也不得改成裸指针。`Drop::drop` 先销毁 iterator，随后 Rust 自动 drop `_collection`，保证 C++ map 晚于 iterator 释放。

- [ ] **步骤 5：实现单表 properties 和 numeric getters**

增加：

```rust
pub struct TableProperties {
    inner: NonNull<ffi::rust_rocksdb_table_properties_t>,
}

impl TableProperties {
    unsafe fn from_raw(inner: *mut ffi::rust_rocksdb_table_properties_t) -> Self {
        Self {
            inner: NonNull::new(inner)
                .expect("RocksDB returned a null table properties value"),
        }
    }

    pub fn data_size(&self) -> u64 {
        unsafe { ffi::rust_rocksdb_table_properties_data_size(self.inner.as_ptr()) }
    }

    pub fn num_entries(&self) -> u64 {
        unsafe { ffi::rust_rocksdb_table_properties_num_entries(self.inner.as_ptr()) }
    }

    pub fn index_size(&self) -> u64 {
        unsafe { ffi::rust_rocksdb_table_properties_index_size(self.inner.as_ptr()) }
    }

    pub fn filter_size(&self) -> u64 {
        unsafe { ffi::rust_rocksdb_table_properties_filter_size(self.inner.as_ptr()) }
    }

    pub fn raw_key_size(&self) -> u64 {
        unsafe { ffi::rust_rocksdb_table_properties_raw_key_size(self.inner.as_ptr()) }
    }

    pub fn raw_value_size(&self) -> u64 {
        unsafe { ffi::rust_rocksdb_table_properties_raw_value_size(self.inner.as_ptr()) }
    }

    pub fn num_data_blocks(&self) -> u64 {
        unsafe { ffi::rust_rocksdb_table_properties_num_data_blocks(self.inner.as_ptr()) }
    }

    pub fn num_deletions(&self) -> u64 {
        unsafe { ffi::rust_rocksdb_table_properties_num_deletions(self.inner.as_ptr()) }
    }

    pub fn num_merge_operands(&self) -> u64 {
        unsafe { ffi::rust_rocksdb_table_properties_num_merge_operands(self.inner.as_ptr()) }
    }

    pub fn num_range_deletions(&self) -> u64 {
        unsafe {
            ffi::rust_rocksdb_table_properties_num_range_deletions(self.inner.as_ptr())
        }
    }
}

impl Drop for TableProperties {
    fn drop(&mut self) {
        unsafe {
            ffi::rust_rocksdb_table_properties_destroy(self.inner.as_ptr());
        }
    }
}
```

- [ ] **步骤 6：实现 property map 拷贝**

增加：

```rust
struct UserCollectedPropertiesIter {
    inner: NonNull<ffi::rust_rocksdb_user_collected_properties_iter_t>,
}

impl UserCollectedPropertiesIter {
    unsafe fn from_raw(
        inner: *mut ffi::rust_rocksdb_user_collected_properties_iter_t,
    ) -> Self {
        Self {
            inner: NonNull::new(inner)
                .expect("RocksDB returned a null user properties iterator"),
        }
    }
}

impl Drop for UserCollectedPropertiesIter {
    fn drop(&mut self) {
        unsafe {
            ffi::rust_rocksdb_user_collected_properties_iter_destroy(
                self.inner.as_ptr(),
            );
        }
    }
}

impl TableProperties {
    fn collect_properties(
        iterator: *mut ffi::rust_rocksdb_user_collected_properties_iter_t,
    ) -> HashMap<Vec<u8>, Vec<u8>> {
        let iterator = unsafe { UserCollectedPropertiesIter::from_raw(iterator) };
        let mut result = HashMap::new();

        loop {
            let mut key = std::ptr::null();
            let mut key_len = 0;
            let mut value = std::ptr::null();
            let mut value_len = 0;
            let has_next = unsafe {
                ffi::rust_rocksdb_user_collected_properties_iter_next(
                    iterator.inner.as_ptr(),
                    &raw mut key,
                    &raw mut key_len,
                    &raw mut value,
                    &raw mut value_len,
                )
            };
            if has_next == 0 {
                break;
            }

            let key = unsafe { copy_bytes(key, key_len) };
            let value = unsafe { copy_bytes(value, value_len) };
            result.insert(key, value);
        }

        result
    }

    pub fn user_collected_properties(&self) -> HashMap<Vec<u8>, Vec<u8>> {
        let iterator = unsafe {
            ffi::rust_rocksdb_table_properties_user_collected_properties_iter_create(
                self.inner.as_ptr(),
            )
        };
        Self::collect_properties(iterator)
    }

    pub fn readable_properties(&self) -> HashMap<Vec<u8>, Vec<u8>> {
        let iterator = unsafe {
            ffi::rust_rocksdb_table_properties_readable_properties_iter_create(
                self.inner.as_ptr(),
            )
        };
        Self::collect_properties(iterator)
    }
}
```

- [ ] **步骤 7：增加 compile-fail 生命周期文档**

在 `TablePropertiesCollection::iter` 文档增加：

```rust
/// The iterator cannot outlive the collection.
///
/// ```compile_fail
/// use rust_rocksdb::{TablePropertiesCollection, TablePropertiesCollectionIter};
///
/// fn force_static(
///     collection: &TablePropertiesCollection,
/// ) -> TablePropertiesCollectionIter<'static> {
///     collection.iter()
/// }
/// ```
```

- [ ] **步骤 8：在 crate root 声明和导出模块**

在 `src/lib.rs` 增加：

```rust
pub mod table_properties;
```

并在公共 re-export block 增加：

```rust
table_properties::{
    TableProperties,
    TablePropertiesCollection,
    TablePropertiesCollectionIntoIter,
    TablePropertiesCollectionIter,
},
```

- [ ] **步骤 9：运行格式和编译**

运行：

```powershell
cargo fmt --all -- --check
cargo test --test test_table_properties_read --features multi-threaded-cf --no-run
```

预期：格式通过；测试可能只因 DB methods 尚未实现而失败，不得再出现 TableProperties 类型、bindgen symbol 或 C++ link 错误。

---

### 任务 6：在 DBCommon 暴露读取方法并完成绿灯

**文件：**
- 修改：`src/db.rs:1060-3580`
- 测试：`tests/test_table_properties_read.rs`

- [ ] **步骤 1：导入公开类型**

在 `src/db.rs` 的 crate imports 中加入：

```rust
TablePropertiesCollection,
```

保持当前 rustfmt import 顺序，不手工制造单独风格分组。

- [ ] **步骤 2：实现默认 CF 方法**

在 `impl<T: ThreadMode, D: DBInner> DBCommon<T, D>` 中靠近其他 metadata/property API 的位置增加：

```rust
/// Returns the table properties for every SST file in the default column family.
pub fn get_properties_of_all_tables(&self) -> Result<TablePropertiesCollection, Error> {
    unsafe {
        let collection = ffi_try!(ffi::rust_rocksdb_get_properties_of_all_tables(
            self.inner.inner()
        ));
        TablePropertiesCollection::from_raw(collection)
    }
}
```

- [ ] **步骤 3：实现指定 CF 方法**

增加：

```rust
/// Returns the table properties for every SST file in `cf`.
pub fn get_properties_of_all_tables_cf(
    &self,
    cf: &impl AsColumnFamilyRef,
) -> Result<TablePropertiesCollection, Error> {
    unsafe {
        let collection = ffi_try!(ffi::rust_rocksdb_get_properties_of_all_tables_cf(
            self.inner.inner(),
            cf.inner()
        ));
        TablePropertiesCollection::from_raw(collection)
    }
}
```

- [ ] **步骤 4：运行 focused tests 并确认绿灯**

运行：

```powershell
cargo test --test test_table_properties_read --features multi-threaded-cf
```

预期：`6 passed; 0 failed`。实际测试数量如因后续步骤增加用例而变化，必须全部通过。

- [ ] **步骤 5：运行 doctest，验证 iterator 生命周期示例**

运行：

```powershell
cargo test --doc --features multi-threaded-cf
```

预期：所有 doctest 通过，compile-fail 示例按预期无法编译。

- [ ] **步骤 6：证明测试能发现 API 缺失**

只在工作树中临时把 `src/lib.rs` 的 `TablePropertiesCollection` re-export 注释掉，运行：

```powershell
cargo test --test test_table_properties_read --features multi-threaded-cf --no-run
```

预期：FAIL，出现 unresolved import。立即恢复该单行改动，再运行同一命令，预期通过。恢复后执行：

```powershell
git diff -- src/lib.rs
```

预期：只剩计划中的正式实现差异，没有临时注释残留。

- [ ] **步骤 7：提交只读 API、FFI 和测试**

运行：

```powershell
$files = @(
  'librocksdb-sys/c-api-extensions/c_api_extensions.h'
  'librocksdb-sys/c-api-extensions/c_api_extensions.cc'
  'src/table_properties.rs'
  'src/db.rs'
  'src/lib.rs'
  'tests/test_table_properties_read.rs'
)
git add -- $files

git diff --cached --check
git diff --cached --name-status
git commit -m "feat: expose table properties read APIs"
```

预期：暂存区只包含列出的 6 个文件，提交成功。

---

### 任务 7：运行分层验证并区分代码、基线和环境问题

**文件：**
- 不新增源码。
- 验证：整个 workspace。

- [ ] **步骤 1：运行 focused Windows 验证**

运行：

```powershell
cargo fmt --all -- --check
cargo test --test test_table_properties_read --features multi-threaded-cf
cargo test --doc --features multi-threaded-cf
cargo clippy --all-targets --features multi-threaded-cf -- -D warnings
git diff --check actual-upstream/master..HEAD
```

预期：全部退出码 `0`。

- [ ] **步骤 2：单独覆盖 GitHub Actions 使用的 Windows MSVC 路径**

运行：

```powershell
cargo +1.91.0-x86_64-pc-windows-msvc test `
  --package rust-librocksdb-sys `
  --target x86_64-pc-windows-msvc `
  --no-run
cargo +1.91.0-x86_64-pc-windows-msvc test `
  --test test_table_properties_read `
  --features multi-threaded-cf `
  --target x86_64-pc-windows-msvc
```

预期：两条命令退出码 `0`。如果缺少 MSVC 1.91、Visual Studio Build Tools、
LLVM 或系统依赖，记录为环境缺口；MinGW 验证通过不能替代该门禁，也不能据此
宣称完整 Windows 兼容。

- [ ] **步骤 3：运行 workspace Windows 验证**

运行：

```powershell
cargo test --workspace --features multi-threaded-cf
```

预期：全部测试通过，失败数为 `0`。

- [ ] **步骤 4：运行 focused WSL/Linux 验证**

运行：

```bash
cd /mnt/d/test/github/review/rust-rocksdb-maintenance
cargo fmt --all -- --check
cargo test --test test_table_properties_read --features multi-threaded-cf
cargo test --doc --features multi-threaded-cf
cargo clippy --all-targets --features multi-threaded-cf -- -D warnings
```

预期：全部退出码 `0`。

- [ ] **步骤 5：运行 workspace WSL/Linux 验证**

运行：

```bash
cd /mnt/d/test/github/review/rust-rocksdb-maintenance
cargo test --workspace --features multi-threaded-cf
```

预期：全部测试通过，失败数为 `0`。

- [ ] **步骤 6：检查 extension 同时被 bundled/system 框架引用**

运行：

```powershell
rg -n 'c-api-extensions/c_api_extensions.cc|build_for_system_backend|c_api_extensions.h' librocksdb-sys/build.rs
```

预期：vendored source list、system backend build 和 bindgen header 三条路径均有匹配。

如果当前机器没有 system RocksDB，不宣称 system backend 已运行通过；将其明确记录为后续 backend 验证计划的环境缺口。

- [ ] **步骤 7：最终提交和工作树对账**

运行：

```powershell
git status --short --branch
git log --oneline --decorate actual-upstream/master..HEAD
git diff --stat actual-upstream/master..HEAD
git diff --check actual-upstream/master..HEAD
```

预期：

- 工作树干净。
- 维护线包含设计、计划、baseline 文档和只读 API 两个语义提交。
- 没有 Collector、Factory 或 Kiwi 仓库改动。
- `git diff --check` 退出码 `0`。

---

## 完成后下一计划的输入

本计划完成后，下一份独立计划使用以下已验证输入：

- `TablePropertiesCollection` 和 `TableProperties` 的最终公开签名。
- extension opaque handle 命名和布局。
- bundled/system extension 编译入口。
- default CF 和 named CF 的运行时测试。
- iterator Drop 和所有权验证结果。

下一计划只处理 Collector callback 和 `UserCollectedProperties` 写入，不重复修改本计划已经冻结的读取 API，除非运行证据证明现有合同存在缺陷。
