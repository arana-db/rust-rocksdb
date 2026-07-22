# TableProperties Collector/Factory 安全迁移设计

## 状态

- 日期：2026-07-21
- 目标仓库：`arana-db/rust-rocksdb`
- 实现分支：`codex/kiwi-table-properties-collector`
- 基线：`master@d1e83230353412387667caf0d531b25a0318e2e0`
- 实际上游：`zaidoon1/rust-rocksdb@a27cb5bdbdb74550835ed5820ad02817c9a8c457`
- RocksDB：`3b446089141659fad25328c5ea3e7ed283df46e4`（11.1.2）
- Snappy：`6af9287fbdb913f0794d0148c6aa43b58e63c8e3`（1.2.2）

## 背景

新的 Arana 维护线已经以 `zaidoon1/rust-rocksdb` 最新 `master` 为直接基线，并通过 PR #5 恢复了 TableProperties 读取 API。当前维护线没有落后实际上游：上游 Head 是 Arana `master` 的祖先，双方 RocksDB、Snappy 子模块 SHA 相同；Arana 额外提交仅包含维护基线文档和 TableProperties 读取能力。

Kiwi 目前仍锁定旧提交 `f7abb18c64fac810f3c4736aef833c340396449b`，原因是新维护线尚未提供以下写入 API：

- `TablePropertiesCollector`
- `TablePropertiesCollectorFactory`
- `TablePropertiesCollectorContext`
- `DBEntryType`
- `Options::set_table_properties_collector_factory`

Kiwi 使用这些 API，在 SST 构建过程中把：

```text
LargestLogIndex/LargestSequenceNumber=<log_index>/<sequence_number>
```

写入 user-collected properties。重启后，Kiwi 通过已经迁移的读取 API恢复各 Column Family 的 applied/flushed Raft index。因此 Collector/Factory 是持久化恢复链路的一部分，不能通过删除调用或静默跳过 Collector 来适配新依赖。

旧 `addtableproperties` 实现只能作为行为参考，不能 cherry-pick。它允许共享 Factory 被并发转换成多个 `&mut`，没有隔离 Rust panic 和 C++ exception，依赖 C++对象布局传递 entry type/sequence，并且缺乏可靠的析构、并发和真实 flush 测试。

## 目标

在当前上游基线上重新实现 Kiwi 所需的 TableProperties Collector/Factory 写入链，同时满足以下要求：

1. 保持 Kiwi 使用的模块路径、类型名、方法名和二进制 property 格式。
2. Factory 符合 RocksDB 的 thread-safe 契约，不产生共享 `&mut`。
3. 每个 SST 拥有独立 Collector；Collector 可安全交给 RocksDB 后台线程。
4. Rust panic、callback 失败和 C++ exception 均不得跨越 Rust/C/C++ ABI。
5. callback 中途失败后不得把半成品或缺失关键恢复元数据的 SST 当作成功结果继续运行。
6. Factory、Collector 和临时 C handle 均有单一、可验证的所有权，精确析构一次。
7. bundled RocksDB 完整支持 Collector/Factory；无法证明私有 C wrapper ABI 的 system backend必须fail-closed，不能执行布局猜测。
8. 完成后能够用真实 Kiwi consumer build 和 Raft 持久化恢复测试验证兼容性。

## 非目标

本阶段不做以下工作：

- 不修改 Kiwi 的 `Cargo.toml` 或 `Cargo.lock`。
- 不改变 `LargestLogIndex/LargestSequenceNumber` 的 key 或 `<log_index>/<sequence_number>` value 格式。
- 不重构已经完成的 TableProperties 读取 API。
- 不恢复旧 `rocksdb_ext/` 目录，不修改 RocksDB 或 Snappy 子模块源码。
- 不向 `zaidoon1/rust-rocksdb` 提交任何分支、PR、Issue 或评论。
- 不顺带修复当前仓库中其他 callback（merge operator、compaction filter、event listener、comparator）的 panic 边界。
- 不在本阶段把 Collector API 全面改成新的公开 `Result`/`Option` 模型。
- 不新增 Kiwi 当前不需要的公共 `BlockAdd` 或 `NeedCompact` Rust trait 方法。
- 不在缺少公共 C API或精确 ABI证明的情况下宣称普通 system RocksDB支持Collector/Factory。

## 公共 Rust API

### `DBEntryType`

继续公开：

```rust
pub enum DBEntryType {
    Put = 0,
    Delete = 1,
    SingleDelete = 2,
    Merge = 3,
    RangeDeletion = 4,
    BlobIndex = 5,
    DeleteWithTimestamp = 6,
    WideColumnEntity = 7,
    TimedPut = 8,
    Other = 9,
}
```

C++ 必须先把 RocksDB `EntryType` 映射为稳定的整数值，再按值传给 Rust。Rust 不得通过 opaque pointer、首字段偏移或 enum 底层布局解释 C++ 对象。无法识别的未来值统一映射为 `Other`。

### `TablePropertiesCollector`

保持 Kiwi 当前使用的方法签名，并增加后台线程所需的 `Send + 'static` 约束：

```rust
pub trait TablePropertiesCollector: Send + 'static {
    fn name(&self) -> &CStr;

    fn add(
        &mut self,
        key: &[u8],
        value: &[u8],
        entry_type: DBEntryType,
        seq: u64,
        file_size: u64,
    );

    fn finish(&mut self) -> HashMap<Vec<u8>, Vec<u8>>;

    fn get_readable_properties(&self) -> HashMap<Vec<u8>, Vec<u8>> {
        HashMap::default()
    }
}
```

RocksDB 对单个 table 的 Collector 顺序调用，因此 Collector 不需要 `Sync`。每个 Collector 仍可使用普通 `&mut self` 管理 table-local 状态。

### `TablePropertiesCollectorFactory`

保留关联类型、方法名和返回类型，只做消除并发未定义行为所必需的签名收紧：

```rust
pub trait TablePropertiesCollectorFactory: Send + Sync + 'static {
    type Collector: TablePropertiesCollector;

    fn create(
        &self,
        context: TablePropertiesCollectorContext,
    ) -> Self::Collector;

    fn name(&self) -> &CStr;
}
```

与旧接口相比，已知的源码兼容性调整包括：

`fn create(&mut self, ...)` 改为：

```rust
fn create(&self, ...)
```

此外，Collector 必须满足 `Send + 'static`，
Factory 必须满足 `Send + Sync + 'static`。持有非 `'static` 借用、`Rc` 或其他
非 `Send`/`Sync` 状态的旧实现需要改为线程安全的拥有型状态；Kiwi 当前基于
`CString` 和 `Arc` 的实现满足这些约束。

不能通过隐藏 Mutex 保留错误的 `&mut self` 公共语义。RocksDB 明确要求 Factory 必须 thread-safe，同一 Factory 可以被多个 flush/compaction 后台线程并发调用。

### `TablePropertiesCollectorContext`

保持四个公开字段：

```rust
pub struct TablePropertiesCollectorContext {
    pub column_family_id: u32,
    pub level_at_creation: i32,
    pub num_levels: i32,
    pub last_level_inclusive_max_seqno_threshold: u64,
}
```

C++ 创建 Factory callback 时按值传递四个字段。Rust 不持有 RocksDB context 指针，也不依赖其布局或生命周期。

### Options 注册方法

继续公开：

```rust
pub fn set_table_properties_collector_factory<F>(&mut self, factory: F)
where
    F: TablePropertiesCollectorFactory;
```

方法继续按值接收 Factory，避免扩大 Kiwi 的迁移 Diff。注册成功后，Factory 生命周期由 C++ `shared_ptr` 保持；Rust 临时 handle 不拥有第二份 Factory Box。

Factory wrapper 构造或 Options 注册发生本地分配失败时，setter 必须先回收尚未转移的 Rust Box，再以固定、可诊断的 panic 终止本次配置调用。该 panic 发生在普通 Rust API 调用栈，不发生在 RocksDB callback 或 C ABI 内。

## 所有权与生命周期

### Factory

```text
Rust Options setter
  └─ Box<FactoryState<F>>
       └─ 转移给 C++ Factory adapter
            └─ shared_ptr<TablePropertiesCollectorFactory>
                 ├─ Options clone
                 ├─ ColumnFamilyOptions
                 ├─ DB open
                 └─ RocksDB background table builders
```

规则：

1. Rust Factory Box 只有一个 destructor callback。
2. C++ adapter 析构时调用该 destructor callback一次。
3. Options 保存 `shared_ptr`，复制 Options 只增加 C++引用计数，不复制 Rust Factory Box。
4. 注册用的 opaque C handle自身也使用 RAII；Options 复制完 `shared_ptr` 后，Rust 可立即销毁临时 handle。
5. 注册失败时，临时 handle仍拥有 `shared_ptr`，销毁 handle会正确释放 Factory。
6. Factory 名称在 setter 中调用 `name()` 并复制成 owned `CString`；名称验证发生在进入 C ABI 之前。

### Collector

```text
Factory create callback
  └─ Box<CollectorState<C>>
       └─ C++ Collector adapter
            └─ unique_ptr<TablePropertiesCollector>
                 └─ 单个 SST table build
```

规则：

1. 每次 `Factory::create(&self, context)` 产生一个全新的 Collector。
2. Collector Box 只由对应的 C++ adapter拥有。
3. C++ `unique_ptr` 在 table build 完成、失败或取消时析构 adapter。
4. adapter 析构时调用 Collector destructor callback一次。
5. Collector 名称在 create trampoline 内复制为 owned `CString`，C++ adapter后续不再调用用户的 `name()`。
6. Collector callback 接收的 key/value slice 只在本次调用期间有效；用户不得保存借用引用。

## FFI 结构

扩展继续放在：

- `librocksdb-sys/c-api-extensions/c_api_extensions.h`
- `librocksdb-sys/c-api-extensions/c_api_extensions.cc`

不恢复旧 `rocksdb_ext/c_ext.cc`。bundled backend在统一扩展源中编译完整adapter；system backend在同一扩展源中通过构建宏编译fail-closed stub，不访问`rocksdb_options_t`私有布局。

### Factory adapter

C++ Factory adapter保存：

- Rust Factory state pointer。
- Factory destructor callback。
- Factory create callback。
- 已缓存的 Factory name。
- `shared_ptr` 所有权。

bundled backend的Options注册沿用当前扩展已经使用的C wrapper布局合同：本仓库固定的 RocksDB `db/c.cc` 中 `rocksdb_options_t` 的首字段是 `rocksdb::Options rep`，现有 EventListener 和其他扩展也通过 `reinterpret_cast<Options*>(opt)` 访问该字段。新实现应集中使用一个带精确 vendored commit依据注释的 `RustOptions(rocksdb_options_t*)` helper，避免在多个函数中重复裸 cast。

`rocksdb_options_t` 的字段布局不是 RocksDB公开 C ABI。外部 system library即使版本号或headers显示为11.1，也不能证明它与扩展编译时假设的私有布局一致。因此 system backend不得执行该cast：

- build script为system backend编译不访问Options私有布局的stub。
- stub公开capability查询，并让Options setter在Rust侧以固定消息明确panic。
- setter必须在转移Factory Box或解引用`rocksdb_options_t`之前拒绝，不能泄漏Factory，也不能触发UB。
- system backend仍可使用已经具有独立安全实现的TableProperties读取API。
- 只有未来提供公共C注册入口，或使用能够以构建产物身份证明与扩展同源同ABI的定制system library时，才能单独设计并启用Collector/Factory system支持。

`CreateTablePropertiesCollector` virtual override必须捕获所有 C++ exception。Rust create callback返回三种内部状态：

- 成功：返回 Collector state及其 callback table。
- Rust panic或 Collector name失败：返回 callback-failure 状态。
- C++/Rust wrapper分配失败：进入保守失败路径。

旧公开 trait没有“正常返回 None”的语义，因此 Factory create失败不能静默返回 `nullptr` 并跳过 Kiwi properties。

RocksDB 11.1.2 明确说明：Collector callback返回的非 OK `Status` 当前只会记录日志，除此之外被忽略；`Finish` 返回失败时只是不写入 collected properties，不能保证 table build/flush失败。因此 Factory create panic、Collector构造失败或后台分配异常必须确定性终止进程。不能依赖 failure Collector或非 OK `Status` 阻止生成缺少Raft恢复元数据的SST。

### Collector adapter

C++ Collector adapter保存：

- Rust Collector state pointer。
- Collector destructor callback。
- `add` callback。
- `finish` callback。
- readable-properties callback。
- 已缓存的 Collector name。
- 固定的 callback失败分类。

`AddUserKey`：

1. 按值传递 entry type、sequence number和file size。
2. 传递仅在 callback期间有效的 key/value pointer和length。
3. Rust callback正常返回时，C++返回 `Status::OK()`。
4. Rust panic、指针/长度不变量失败或 C++ exception时，C++确定性调用 `std::abort()`。
5. 不能只返回非 OK `Status`，因为当前 RocksDB会忽略该状态并继续table build。

`Finish`：

1. Rust `finish()` 在 `catch_unwind` 内执行。
2. 返回的 `HashMap<Vec<u8>, Vec<u8>>` 通过同步 sink callback逐项复制到 C++ `UserCollectedProperties`。
3. C++ map insertion 的分配异常在 sink内部捕获。
4. 只有全部 entries复制成功，C++ 才把临时 map交换到 RocksDB 输出参数。
5. 任意 panic、复制错误或 C++ exception都确定性调用 `std::abort()`，不得让 RocksDB继续生成缺少或只包含部分properties的SST。

`GetReadableProperties`：

1. 使用同一同步 sink复制模型。
2. panic或复制错误返回空 map。
3. readable properties仅用于诊断，不得影响已完成的持久化结果。

`NeedCompact` 使用 RocksDB基类默认行为或固定返回 `false`；本阶段不调用用户 Rust代码。

`BlockAdd` 使用 RocksDB基类默认实现；本阶段不扩展旧公开 Rust trait。

## Panic、错误和异常边界

### Rust panic

每个 Rust `extern "C"` trampoline都必须使用：

```rust
catch_unwind(AssertUnwindSafe(|| { ... }))
```

策略如下：

| 位置 | 处理方式 |
|---|---|
| Factory create | 确定性 abort，禁止静默跳过 Collector |
| Collector name | 确定性 abort，禁止创建无有效身份的 Collector |
| Collector add | 确定性 abort，禁止继续构建不可信 SST |
| Collector finish | 确定性 abort，禁止写入缺失或半成品 properties |
| readable properties | 返回空 map |
| Factory/Collector destructor | 捕获并吞掉 panic，绝不再次析构，绝不越过 ABI |

Factory name在普通 setter调用中预先验证，不位于后台 callback边界。

### C++ exception

以下位置全部使用 `noexcept` 和内部 `try/catch (...)`：

- C ABI Factory handle创建/销毁。
- Options 注册入口。
- Factory `CreateTablePropertiesCollector` override。
- Collector `AddUserKey` override。
- Collector `Finish` override。
- Collector `GetReadableProperties` override。
- Factory/Collector adapter析构。
- `shared_ptr`、`unique_ptr`、`std::string` 和 properties map分配/复制。

异常不能逃入 RocksDB。Factory create、Collector add/finish以及持久化properties复制中的异常在catch边界内转为确定性abort；readable-properties中的异常返回空map；析构中的异常被吞掉。RocksDB 对 Collector/Factory 的约束明确指出，逃逸异常可能导致数据丢失、未报告损坏或死锁。

### 错误消息所有权

callback 不跨 Rust/C++ allocator传递动态错误字符串。Rust callback返回固定状态码；C++ 可以在abort前输出固定、上下文明确的诊断，例如：

```text
Rust table properties collector callback panicked
Rust table properties collector add callback failed
Failed to copy Rust table properties
Rust table properties collector factory callback failed
```

这样避免 `CString::into_raw` 与 C++ `free()`/CRT 不匹配，也消除错误字符串泄漏。

## 二进制数据规则

- key、value、property key和property value都按任意字节序列处理。
- 不使用 `CStr`、UTF-8 或有损 `String` 转换处理业务数据。
- 空 key/value合法；长度为 0 时不得对空指针调用 `slice::from_raw_parts`。
- C++ 在 callback返回前复制所有 Rust property字节。
- Rust map/vector被释放后，SST 中的数据必须保持有效。
- duplicate property key遵循 Rust `HashMap` 的唯一 key语义；传给 C++ 前每个 key最多出现一次。

## 文件边界

预计新增或修改：

- 新增 `src/table_properties_collector.rs`
  - `DBEntryType`
  - Collector trait
  - Collector callback state和trampolines
  - panic、fail-fast和properties sink逻辑
- 新增 `src/table_properties_collector_factory.rs`
  - Factory trait
  - context
  - Factory callback state和trampolines
  - Factory/Collector创建所有权交接
- 修改 `src/db_options.rs`
  - `Options::set_table_properties_collector_factory`
- 修改 `src/lib.rs`
  - 公开两个兼容模块
- 修改 `librocksdb-sys/c-api-extensions/c_api_extensions.h`
  - opaque handles、callback typedef和注册接口
- 修改 `librocksdb-sys/c-api-extensions/c_api_extensions.cc`
  - C++ Factory/Collector adapters和异常边界
- 修改 `librocksdb-sys/build.rs`
  - 区分bundled完整实现与system fail-closed stub
  - 向Rust暴露可验证的capability cfg或FFI查询
- 新增 `tests/test_table_properties_collector_factory.rs`
  - 真实 RocksDB flush、并发、panic、drop和读写闭环测试

不允许仅凭system RocksDB版本/header字符串启用私有布局cast。

## 测试设计

### API 与数据闭环

1. 默认 CF 写入多种 entry，执行真实 flush。
2. 验证 `add` 收到 key、value、entry type、sequence和file size。
3. `finish` 写入二进制 user properties。
4. 使用当前 `get_properties_of_all_tables[_cf]` 读取并核对结果。
5. 验证 readable properties。
6. 覆盖空 key/value、包含 NUL 的二进制数据和最大边界数值。
7. 覆盖未知 entry type到 `Other` 的保守映射。

### Context 与多 CF

1. 验证默认 CF和命名 CF的 `column_family_id`。
2. 验证 level、num_levels和threshold字段按值传递。
3. 设置 `max_background_jobs > 1`，并发 flush多个 CF。
4. 使用 barrier/atomics证明 Factory create可以重叠执行。
5. 验证每次 create产生不同 Collector实例，结果不串扰。

### 生命周期与 Drop

分别用 `Arc<AtomicUsize>` 统计 Factory和Collector drop：

- Options正常 drop。
- Options clone。
- 原始 Options先于 DB drop。
- 多 CF共享 Factory。
- 正常 flush。
- flush失败。
- DB open失败。
- DB关闭时仍有后台工作。

这些非abort路径中的每个已创建对象必须精确drop一次，不能提前drop、double drop或泄漏。

### Panic 与 fail-fast

分别注入：

- Factory create panic。
- Collector name panic。
- Collector add panic。
- Collector finish panic。
- readable-properties panic。
- Factory/Collector Drop panic。

验收：

- panic不穿越 ABI。
- Factory create、Collector add/finish和持久化properties复制失败会确定性abort。
- 使用subprocess断言确定的非零退出状态，且不会报告flush成功。
- 重新打开测试目录时，不得观察到一个被当作成功完成且缺少关键property的新SST。
- SST 中不包含半成品property。
- readable-properties panic只返回空结果。
- abort场景不要求DB close、C++栈展开或Factory/Collector destructor callback执行。
- Factory/Collector Drop panic在独立的非abort测试中验证：析构callback只进入一次，panic被捕获且不越过ABI。

需要验证进程级行为的场景使用 subprocess test，避免一个故意 abort/panic场景终止整个测试进程。

### 编译期线程门禁

使用 compile-fail测试或等效静态断言确认：

- 捕获 `Rc<RefCell<_>>` 的 Factory不能注册。
- 非 `Send` Collector不能编译。
- 捕获非 `'static` 引用的 Factory/Collector不能编译。
- 使用 `Arc<Mutex<_>>`、`Arc<RwLock<_>>` 或 atomics 的实现可以编译。

### 平台与动态检测

- Linux/WSL：格式、focused tests、Clippy、workspace tests。
- Windows MSVC：构建与 Collector/Factory集成测试。
- GitHub CI：Linux、ARM、macOS、Windows、ASan和`multi-threaded-cf`。
- coroutines 独立运行，不并入 blanket `--all-features`。
- 条件允许时增加 UBSan；Miri仅检查纯 Rust state/trampoline，不能替代真实 C++后台 callback测试。
- system RocksDB backend至少完成stub编译、capability查询和setter fail-closed测试；它不得访问Options私有布局或宣称Collector/Factory可用。

## Kiwi 消费方迁移

Collector/Factory PR合并后，Kiwi单独提交：

1. 把 Factory实现的 `create(&mut self, ...)` 改成 `create(&self, ...)`。
2. 将 `rust-rocksdb` revision更新到包含读取和写入 API 的正式 merge SHA。
3. 使用 Cargo正常更新 lockfile，不手工替换 source SHA。
4. 明确 Kiwi最低 Rust版本为1.91。
5. 验证：
   - `cargo check -p raft --all-targets --locked`
   - `cargo test -p raft --lib table_properties --locked`
   - `cargo test -p raft --test logindex_integration --locked`
   - 多 CF flush。
   - 关闭并重新打开同一路径，从 SST恢复 applied/flushed index。
   - workspace build、Clippy和tests。
6. 验证旧 SST property与新实现兼容，不改变持久化格式。

## 拒绝的替代方案

### 直接 cherry-pick旧实现

拒绝。旧实现存在共享 `&mut Factory`、panic跨ABI、C++ exception逃逸、布局猜测、错误字符串所有权和真实flush测试缺失等问题。

### 保留 `create(&mut self)` 并在内部加锁

拒绝。虽然可以用 Mutex序列化 callback，但公共 trait仍表达错误的独占访问语义，增加重入和锁顺序风险，并掩盖 RocksDB明确的thread-safe合同。

### 本阶段全面改成公开 `Result`/`Option`

拒绝。完整错误模型长期更清晰，但会扩大 Kiwi和其他调用方的迁移范围，而且当前 RocksDB仍会忽略 Collector callback返回的非 OK `Status`。本阶段通过内部状态码识别panic/失败，并在关键持久化路径fail-fast，同时保持旧公开 API主体兼容。

### Factory create失败时返回 `nullptr` 或 failure `Status`

拒绝。旧公开 API没有“有意不创建 Collector”的语义；对 Kiwi而言，静默跳过 Collector会生成缺少Raft恢复元数据的SST。当前 RocksDB又会忽略 Collector callback的非 OK `Status`，所以 failure Collector不能可靠阻断table build。关键写入callback失败必须fail-fast。

## 验收标准

实现只有同时满足以下条件才可提交合并判断：

1. Kiwi所需公开模块、类型和方法恢复；唯一必要源码调整是 Factory `&mut self` 到 `&self`。
2. Factory具备 `Send + Sync + 'static`，Collector具备 `Send + 'static`。
3. 无 Rust panic或 C++ exception可跨越ABI。
4. callback失败不会写入部分properties，也不会静默省略Collector；关键写入路径失败会确定性abort。
5. Factory/Collector在正常、可恢复失败、Options clone和多CF路径精确析构一次；abort路径不承诺析构。
6. 真实flush后，读取API能取回准确的binary user/readable properties。
7. 并发多CF测试证明Factory共享安全、Collector实例隔离。
8. bundled backend包含完整Collector/Factory扩展；system backend包含可测试的fail-closed stub，且不会执行Options私有布局cast。
9. Linux、Windows MSVC和GitHub多平台门禁按可用环境执行；任何环境缺口单独记录，不伪装成源码通过。
10. Kiwi consumer build、logindex integration和关闭/重开恢复测试通过后，才允许更新Kiwi生产pin。
