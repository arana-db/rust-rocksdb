# TableProperties Collector/Factory 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 在当前 Arana 维护线上恢复 Kiwi 所需的 TableProperties Collector/Factory 写入 API，同时封住 Factory 并发别名、Rust panic、C++ exception、所有权泄漏和 system RocksDB 私有布局风险。

**架构：** Factory 使用 `Send + Sync + 'static` 和 C++ `shared_ptr`；每个 SST 的 Collector 使用 `Send + 'static` 和 C++ `unique_ptr`。关键写入 callback 失败时 fail-fast，因为 RocksDB 11.1.2 会忽略非 OK Collector `Status`；system backend只提供capability=false stub。

**技术栈：** Rust 1.91、C++20、RocksDB 11.1.2、bindgen、Cargo integration tests、WSL/Linux、Windows MSVC。

---

## 基线

- Worktree：`D:\test\github\review\rust-rocksdb-collector-factory`
- 分支：`codex/kiwi-table-properties-collector`
- 基线：`d1e83230353412387667caf0d531b25a0318e2e0`
- 规格：`docs/superpowers/specs/2026-07-21-table-properties-collector-factory-design.md`
- 旧实现仅作参考：`origin/addtableproperties@f7abb18c64fac810f3c4736aef833c340396449b`
- 不修改 RocksDB/Snappy submodule。
- 不修改 Kiwi pin。
- 不向 `zaidoon1/rust-rocksdb` 写入。
- 每个任务独立提交并执行 focused test；P0/P1 修复后必须重跑。

## 文件边界

- 创建 `src/table_properties_collector.rs`
- 创建 `src/table_properties_collector_factory.rs`
- 修改 `src/lib.rs`
- 修改 `src/db_options.rs`
- 修改 `librocksdb-sys/c-api-extensions/c_api_extensions.h`
- 修改 `librocksdb-sys/c-api-extensions/c_api_extensions.cc`
- 修改 `librocksdb-sys/build.rs`
- 创建 `tests/test_table_properties_collector_factory.rs`

---

### 任务 1：公共 Rust API 与线程合同

**文件：**
- 创建：`src/table_properties_collector.rs`
- 创建：`src/table_properties_collector_factory.rs`
- 修改：`src/lib.rs:99-112`
- 创建：`tests/test_table_properties_collector_factory.rs`

- [ ] **步骤 1：编写失败的公共 API 测试**

测试定义 Kiwi 风格 Collector/Factory，并要求：

```rust
fn assert_collector<T: TablePropertiesCollector>() {}
fn assert_factory<T: TablePropertiesCollectorFactory>() {}
```

Factory 实现必须使用 `create(&self, context)`。运行：

```powershell
cargo test --test test_table_properties_collector_factory public_traits_accept_thread_safe_types --no-run
```

预期：模块尚不存在而编译失败。

- [ ] **步骤 2：实现 `DBEntryType`**

使用 `#[repr(u8)]`，保留 Put=0 到 TimedPut=8，所有未知整数映射到 `Other`。测试0..8及255。

- [ ] **步骤 3：实现 Collector trait**

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
        HashMap::new()
    }
}
```

- [ ] **步骤 4：实现 Context 和 Factory trait**

```rust
pub struct TablePropertiesCollectorContext {
    pub column_family_id: u32,
    pub level_at_creation: i32,
    pub num_levels: i32,
    pub last_level_inclusive_max_seqno_threshold: u64,
}

pub trait TablePropertiesCollectorFactory: Send + Sync + 'static {
    type Collector: TablePropertiesCollector;
    fn create(&self, context: TablePropertiesCollectorContext) -> Self::Collector;
    fn name(&self) -> &CStr;
}
```

- [ ] **步骤 5：公开模块并验证**

```powershell
cargo fmt --all -- --check
cargo test --test test_table_properties_collector_factory public_traits_accept_thread_safe_types --no-run
git diff --check
```

- [ ] **步骤 6：Lore commit**

提交主题：`Define thread-safe table properties collector contracts`。

---

### 任务 2：C ABI、C++ adapter 与 system capability

**文件：**
- 修改：`librocksdb-sys/c-api-extensions/c_api_extensions.h`
- 修改：`librocksdb-sys/c-api-extensions/c_api_extensions.cc`
- 修改：`librocksdb-sys/build.rs`
- 修改：`tests/test_table_properties_collector_factory.rs`

- [ ] **步骤 1：添加 capability 失败测试**

要求bundled backend返回1；system backend返回0。先运行 `--no-run`，确认符号缺失。

- [ ] **步骤 2：声明 C ABI**

声明三个 opaque handle：

- `rust_rocksdb_table_properties_collector_t`
- `rust_rocksdb_table_properties_collector_factory_t`
- `rust_rocksdb_user_collected_properties_sink_t`

声明：

- capability query
- Collector create/destroy
- Factory create/destroy
- Options register
- binary property sink add
- add/finish/readable/create callbacks

key/value使用pointer+length；entry type、sequence、file size和context字段按整数值传递。

- [ ] **步骤 3：实现 bundled Collector adapter**

`AddUserKey`、`Finish`、`GetReadableProperties`、`Name` 和析构全部 `noexcept` 并catch-all。

- add callback失败：`std::abort()`
- finish或property复制失败：`std::abort()`
- readable失败：返回空map
- name：返回构造时缓存字符串
- destructor：只调用一次Rust destructor callback

- [ ] **步骤 4：实现 bundled Factory adapter**

Factory state由 `shared_ptr` 持有；create callback返回带 `unique_ptr` 的Collector临时handle，C++移动所有权后销毁handle。create为null或异常时abort。

Options注册仅bundled执行：

```cpp
RustOptions(opt)->table_properties_collector_factories.emplace_back(factory->rep);
```

- [ ] **步骤 5：实现 system fail-closed stub**

build.rs：

```rust
// vendored
cfg.define("RUST_ROCKSDB_COLLECTOR_FACTORY_SUPPORTED", Some("1"));
// system
cfg.define("RUST_ROCKSDB_COLLECTOR_FACTORY_SUPPORTED", Some("0"));
```

system路径不得出现 `reinterpret_cast<Options*>(opt)`；create/register返回失败，capability返回0。

- [ ] **步骤 6：验证并提交**

```powershell
cargo fmt --all -- --check
cargo test --test test_table_properties_collector_factory bundled_backend_reports_support --no-run
git diff --check
```

提交主题：`Bridge table properties collectors through safe C++ adapters`。

---

### 任务 3：Rust trampolines 与 Options setter

**文件：**
- 修改：`src/table_properties_collector.rs`
- 修改：`src/table_properties_collector_factory.rs`
- 修改：`src/db_options.rs:1787-1800`
- 修改：`tests/test_table_properties_collector_factory.rs`

- [ ] **步骤 1：先写真实flush失败测试**

注册 Kiwi 风格Factory，put多条记录并flush，通过现有 `get_properties_of_all_tables` 读取 `LargestLogIndex/LargestSequenceNumber`。预期setter尚不存在而失败。

- [ ] **步骤 2：实现Collector trampolines**

所有 `unsafe extern "C"` callback使用：

```rust
catch_unwind(AssertUnwindSafe(|| { ... }))
```

规则：

- null+0转换为空slice
- null+非0返回失败码
- add使用 `&mut Collector`
- finish将HashMap逐项同步复制进C++ sink
- readable使用共享借用，panic返回失败码
- destructor只 `Box::from_raw` 一次并catch panic

- [ ] **步骤 3：实现Factory trampoline**

Factory state只构造 `&F`。按值创建Context，调用 `create(&self)`，复制Collector name，再创建C handle。panic返回null，由C++ fail-fast。

- [ ] **步骤 4：实现 Options setter**

顺序必须是：

1. capability=0时立即panic，尚未调用Factory name或转移Box
2. 复制Factory name
3. Box转raw
4. Factory handle创建失败时重建Box
5. Options注册失败时销毁handle
6. 注册成功后销毁临时handle，Options的shared_ptr继续持有

固定system错误：`TablePropertiesCollectorFactory requires the bundled RocksDB backend`。

- [ ] **步骤 5：验证读写闭环并提交**

```powershell
cargo test --test test_table_properties_collector_factory writes_binary_properties_after_real_flush -- --nocapture
cargo test --test test_table_properties_read --features multi-threaded-cf
cargo fmt --all -- --check
git diff --check
```

提交主题：`Register thread-safe Rust table properties factories`。

---

### 任务 4：Context、并发和生命周期测试

**文件：**
- 修改：`tests/test_table_properties_collector_factory.rs`

- [ ] **步骤 1：验证callback参数**

覆盖key/value、Put/Delete/SingleDelete/Merge/RangeDeletion、sequence number、file size，以及Context四字段。

- [ ] **步骤 2：验证多CF**

设置 `max_background_jobs=4`，多个CF并发写入和flush。每个Collector分配唯一ID，断言properties不串扰。

- [ ] **步骤 3：验证线程合同**

使用Barrier/atomics记录Factory create同时进入；如果runner无法稳定产生重叠，不用sleep伪造，保留编译期 `Send + Sync` 门禁和多实例隔离证据。

- [ ] **步骤 4：验证精确drop**

统计：

- Options clone
- 原Options先drop
- 多CF
- 正常flush
- DB open失败
- DB关闭时有后台任务

非abort路径中Factory drop一次，每个Collector drop一次。

- [ ] **步骤 5：验证并提交**

```powershell
cargo test --test test_table_properties_collector_factory context_ -- --nocapture
cargo test --test test_table_properties_collector_factory concurrent_ -- --nocapture
cargo test --test test_table_properties_collector_factory drop_ -- --nocapture
cargo clippy --test test_table_properties_collector_factory -- -D warnings
cargo fmt --all -- --check
```

提交主题：`Verify collector context concurrency and ownership`。

---

### 任务 5：Panic fail-fast 和 system stub

**文件：**
- 修改：`tests/test_table_properties_collector_factory.rs`
- 必要时修改前述Rust/C++实现文件

- [ ] **步骤 1：建立subprocess入口**

父进程使用 `current_exe()`、`--exact collector_subprocess_entry` 和环境变量选择case及DB路径。

- [ ] **步骤 2：验证关键panic**

分别注入Factory create、Collector name、add和finish panic。断言：

- child非成功退出
- 未输出 `FLUSH_SUCCEEDED`
- 父进程能重新打开DB路径
- 没有被当作成功完成但缺少关键property的新SST
- 不要求abort路径执行DB close或drop callback

- [ ] **步骤 3：验证可恢复panic**

- readable-properties panic：进程存活，user properties正确，readable map为空
- Factory/Collector Drop panic：destructor callback只进入一次，panic不越过ABI

- [ ] **步骤 4：验证system stub**

有system RocksDB时验证capability=0和setter在ownership transfer前panic；没有环境时记录未运行，并源码确认stub路径无Options cast。

- [ ] **步骤 5：验证并提交**

```powershell
cargo test --test test_table_properties_collector_factory fail_fast_ -- --nocapture
cargo test --test test_table_properties_collector_factory readable_panic_ -- --nocapture
cargo test --test test_table_properties_collector_factory drop_panic_ -- --nocapture
cargo fmt --all -- --check
cargo clippy --test test_table_properties_collector_factory -- -D warnings
git diff --check
```

提交主题：`Prove collector callbacks fail closed across FFI`。

---

### 任务 6：完整验证和Consumer合同

- [ ] **步骤 1：focused验证**

```powershell
cargo fmt --all -- --check
cargo test --test test_table_properties_collector_factory -- --nocapture
cargo test --test test_table_properties_read --features multi-threaded-cf
cargo clippy --test test_table_properties_collector_factory -- -D warnings
git diff --check
```

- [ ] **步骤 2：WSL workspace验证**

```bash
cargo test --workspace --features multi-threaded-cf
cargo clippy --workspace --all-targets --features multi-threaded-cf -- -D warnings
```

coroutines单独运行，不使用blanket `--all-features`。

- [ ] **步骤 3：Windows MSVC验证**

```powershell
cargo +1.91.0-x86_64-pc-windows-msvc test --test test_table_properties_collector_factory --features multi-threaded-cf
```

缺少工具链时记录环境错误，不用MinGW替代。

- [ ] **步骤 4：只读核对Kiwi**

核对 `D:\test\github\kiwi\src\raft\src\table_properties.rs`。唯一必要源码变化应为 `create(&mut self)` 到 `create(&self)`；模块、方法和property格式必须保持。

- [ ] **步骤 5：两阶段审查**

先规格符合性，再代码质量；重点检查raw pointer、catch_unwind、noexcept、RocksDB忽略Status后的fail-fast、Factory并发、system stub和silent metadata loss测试。

- [ ] **步骤 6：收尾**

分开报告：

- 通过的测试
- GitHub CI
- baseline/环境失败
- system未运行范围
- Windows MSVC覆盖
- Kiwi尚未更新pin的原因
