# Kiwi rust-rocksdb 维护线设计

## 1. 目标

在 `arana-db/rust-rocksdb` 中建立一条独立、可持续同步的 Kiwi 维护线：

1. 以 `zaidoon1/rust-rocksdb` 实施时的最新 `master` 为源码和 Git 历史基线。
2. 不把新维护线与长期分叉的 Arana `master` 机械合并。
3. 以旧 Arana Kiwi pin 为行为参考，按语义重新移植 Kiwi 实际依赖的最小 TableProperties 能力。
4. 在移植过程中修复旧实现已经确认的线程安全、panic/unwind、所有权、system backend 和工程门禁问题。
5. 最终由 Kiwi 固定到经过验证的 Arana commit SHA，不直接依赖外部 upstream。

## 2. 已固定的来源

### 2.1 新维护线基线

- 仓库：`https://github.com/zaidoon1/rust-rocksdb.git`
- Remote：`actual-upstream`
- Push URL：`DISABLED`
- `master` Head：`a27cb5bdbdb74550835ed5820ad02817c9a8c457`
- `rust-rocksdb`：`0.51.0`
- `librocksdb-sys`：`0.47.1+11.1.2`
- MSRV：Rust `1.91`
- RocksDB submodule：`3b446089141659fad25328c5ea3e7ed283df46e4`，RocksDB `11.1.2`
- Snappy submodule：`6af9287fbdb913f0794d0148c6aa43b58e63c8e3`，Snappy `1.2.2`

### 2.2 旧 Arana 定制来源

- 旧定制直接基线：`4f973bf6d94d8a3b32a39697b63092de08106974`
- Kiwi 当前 pin：`f7abb18c64fac810f3c4736aef833c340396449b`
- 定制提交数：7
- 定制文件数：12
- 差异规模：约 `+1979/-5`

旧提交按顺序为：

1. `55459dab9aa01fa4cbbed100ca9cdd2dbef87ed5`，TablePropertiesCollectorFactory 和 Options 接入。
2. `3201c73e85f042772ef17eab6a05a04bff989eec`，`get_readable_properties`。
3. `faff47f10ab83c3e51a7dc55f13ac437008adb36`，readable properties 测试。
4. `6d543c713dae14411c3986ba032e381b36b08fab`，本地 worktree ignore，不属于产品能力。
5. `30df7bffb0a3f7c4524398b61d0e3ff9bfd7045a`，TableProperties 数据模型和读取 API。
6. `5e09f68c776776780bee22ede7a3c99a236855ab`，构建修补。
7. `f7abb18c64fac810f3c4736aef833c340396449b`，构建修补。

这些提交只作为需求、行为和测试参考，不直接 cherry-pick。新维护线不得继承无语义提交、旧目录布局或已知不安全实现。

## 3. 分支和远端模型

### 3.1 本地实施分支

实施分支固定为：

```text
codex/kiwi-maintenance-rebuild
```

该分支直接从 `actual-upstream/master` 的精确 SHA `a27cb5bd` 创建。

### 3.2 远端稳定分支

验证完成并获得 push 授权后，在 Arana 仓库建立：

```text
kiwi-maintenance
```

该分支是 Kiwi fork 的稳定 Base。后续 TableProperties、FFI 修复、upstream 同步和版本升级 PR 均以它为 Base，不以旧 `master` 或 `addtableproperties` 为 Base。

### 3.3 远端边界

- `actual-upstream` 只允许 fetch。
- 禁止向 `zaidoon1/rust-rocksdb` push 分支或 tag。
- 禁止向 `zaidoon1/rust-rocksdb` 创建 PR、Issue、Discussion 或评论。
- 所有维护提交、PR、CI 和发布状态只存在于 `arana-db/rust-rocksdb`。
- 在 `kiwi-maintenance` 建立、保护规则和 checks 验证完成前，不关闭或删除现有证据分支。

## 4. 不采用的集成方式

### 4.1 不向旧 Arana master 合并完整 upstream

旧 `master` 与新基线的共同祖先停在 2024-01-29。两侧分别独有 112 和 301 个提交，普通文件三方探针已经得到 49 个冲突文件和 172 个文本冲突块，RocksDB submodule 也已分叉。

逐文件解决这些冲突会生成未经设计的混合基线，因此不采用。

### 4.2 不直接 cherry-pick 旧 7 个定制提交

旧提交依赖已经废弃的 `rocksdb_ext/c_ext.*` 布局和旧 `build.rs`，并包含以下已知问题：

- Factory trait 缺少 `Send + Sync`。
- `create(&mut self)` 允许 FFI 从共享裸指针构造多个 `&mut F`。
- Rust panic 可能跨越 C ABI。
- system RocksDB backend 声明扩展符号但不编译实现。
- owning iterator 使用不必要的 `std::mem::zeroed()`。
- 缺少当前 upstream 的构建和 CI 约束。

新实现必须按当前架构重新组织，不能先引入问题再修补。

### 4.3 不修改 RocksDB submodule

所有新增 C/C++ 接口必须位于 `librocksdb-sys/c-api-extensions/`。不得向 `librocksdb-sys/rocksdb` 写入文件或创建补丁提交。

## 5. 实施阶段和提交边界

### 5.1 阶段 0：维护基线和来源文档

建立可审计基线提交，仅增加 Arana 自有维护文档，不修改 upstream 业务实现。

文档必须记录：

- upstream、submodule、版本和 MSRV 的完整 SHA。
- Arana 旧定制基线和 Kiwi pin。
- 禁止向 external upstream 写入的远端边界。
- 后续移植阶段和验收命令。

基线提交前后必须证明源码树除维护文档外与 `a27cb5bd` 一致。

### 5.2 阶段 1：TableProperties 只读模型

先恢复读取现有 SST properties 的路径，不在同一提交引入用户 callback。

新增或恢复：

- `TableProperties`
- `TablePropertiesCollection`
- collection iterator
- `user_collected_properties`
- `readable_properties`
- `DB::get_properties_of_all_tables()`
- `DB::get_properties_of_all_tables_cf()`

只读模型必须保持二进制值，不把 user-collected property 强制转换为 UTF-8。字符串转换只能作为显式的便利接口或诊断显示。

所有权设计要求：

- Collection 独占底层 C++ collection handle。
- Iterator 的生命周期不得超过 Collection。
- Owning iterator 通过正常 Rust 所有权移动保存 Collection，不使用 `std::mem::zeroed()`、`forget()` 或伪造无效值。
- 每个 C/C++ handle 必须有唯一释放责任。
- 提前 drop、部分消费、空 collection 和完整消费均不得泄漏或 double free。

### 5.3 阶段 2：TablePropertiesCollector

恢复 Kiwi 写入 SST user-collected properties 所需的最小 Collector：

- `TablePropertiesCollector`
- `DBEntryType`
- `add`
- `finish`
- `get_readable_properties`
- `name`

FFI 符号统一使用：

```text
rust_rocksdb_*
```

callback 边界要求：

- Rust panic 不得穿越 C ABI。
- 能返回 RocksDB `Status` 的 callback 使用 `catch_unwind`，把 panic 转换为包含上下文的错误状态。
- 不能返回 `Status` 的 callback 只能访问预构造、稳定、不会 panic 的数据。
- trait 文档明确实现不得主动 panic。
- C++ wrapper 不允许异常逃出 RocksDB callback。

### 5.4 阶段 3：线程安全 Factory

恢复：

- `TablePropertiesCollectorFactory`
- `TablePropertiesCollectorContext`
- `Options::set_table_properties_collector_factory`

公开接口固定为等价于：

```rust
pub trait TablePropertiesCollectorFactory: Send + Sync {
    type Collector: TablePropertiesCollector;

    fn create(
        &self,
        context: TablePropertiesCollectorContext,
    ) -> Self::Collector;

    fn name(&self) -> &CStr;
}
```

FFI 从共享 factory 指针只能构造 `&F`，禁止构造 `&mut F`。Factory 的 Rust 所有权由 Options/C++ wrapper 明确定义，在 RocksDB 不再调用 callback 后释放一次。

### 5.5 阶段 4：bundled 和 system backend

当前 upstream 已有 `c-api-extensions` 构建框架，新能力必须接入该框架。

bundled backend：

- 将 extension `.cc` 加入已有 `cc::Build` source list。
- bindgen 以 extension header 为入口，并传递包含 upstream `rocksdb/c.h` 的声明。
- 构建只向 Cargo `OUT_DIR` 写入产物。

system backend：

- 将 extension 编译为独立静态 archive。
- 与用户提供的 system RocksDB 一起链接。
- 对 RocksDB 版本和需要的 C++ header 做构建期验证。
- 无法支持的平台必须在 build/configure 阶段返回清晰错误，不得延迟到最终 linker 报未定义符号。

两条 backend 都不能要求 Git 存在，也不能改写源码树。

### 5.6 阶段 5：CI 和工程清理

新维护线必须保留 upstream 的现有 CI，并增加 Arana 定制能力对应的验证矩阵。

提交历史必须按语义拆分。不得保留 `cargo build success`、`feat add table pro` 等无法说明行为的提交标题。

旧 `.gitignore` 中仅服务某次本地工具的条目不自动移植。只有当前维护工作流确实需要且不会污染仓库语义时才增加。

### 5.7 阶段 6：Kiwi 集成

fork 自身通过后，在 Kiwi 独立分支更新固定 revision。Kiwi 集成不与 fork 实现混在同一仓库或同一提交中。

## 6. 测试设计

### 6.1 基线门禁

在引入 Arana 定制前执行 upstream 当前支持的基础验证，记录环境和完整输出。至少包括：

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --features multi-threaded-cf -- -D warnings
cargo test --workspace --features multi-threaded-cf
```

submodule 必须精确匹配基线记录，工作树必须干净。

### 6.2 TableProperties 读取测试

- 空 DB 和空 collection。
- 单个 SST、多个 SST。
- 默认 CF 和多个 CF。
- 空 user property。
- 二进制和非 UTF-8 property。
- property 缺失。
- collection 完整消费。
- iterator 提前 drop。
- 部分消费后 drop。
- Collection 先于借用 iterator 释放必须在编译期被拒绝。

### 6.3 Collector 和 Factory 测试

- `add` 接收 key、value、sequence number、entry type 和文件大小。
- `finish` 写入 Kiwi 需要的 property。
- `get_readable_properties` 结果可读取。
- 多个 CF 同时 flush。
- `max_background_jobs > 1`。
- flush 和 compaction 并行。
- 多线程并发调用同一个 Factory。
- Factory 和 Collector 的 panic/error 路径不会跨越 FFI unwind。
- callback 状态和 C++ handle 只释放一次。

### 6.4 构建矩阵

- bindgen-runtime。
- bindgen-static。
- bundled RocksDB。
- system RocksDB，或明确的构建期拒绝测试。
- 只读源码树。
- 构建环境中没有 Git。
- Windows。
- WSL/Linux。
- upstream CI 已覆盖且 Arana 能继续维护的 macOS、ARM、ASan 和 coroutine 组合。

### 6.5 Kiwi 集成测试

- `src/raft/src/table_properties.rs` 编译和行为。
- `src/raft/src/cf_tracker.rs` 从 SST 恢复 applied/flushed log index。
- 多 CF flush 和状态恢复。
- DB 关闭、重开和恢复。
- RocksDB 10.9.1 生成的旧 SST/custom properties 可由 RocksDB 11.1.2 读取。
- `<log_index>/<sequence_number>` 持久化格式保持不变。
- property 缺失、损坏、多段值和异常字节不会导致错误恢复。

## 7. 验证和完成标准

每个阶段的最小通用门禁：

```bash
cargo fmt --all -- --check
cargo clippy --all-features --workspace -- -D warnings
cargo test --all-features --workspace
git diff --check
```

若 `all-features` 依赖 Folly、coroutines 或特定平台工具链，应使用 upstream 对应 CI 容器和 workflow。环境缺失必须单独记录，不能伪装为代码失败，也不能据此跳过必要验证。

维护线完成必须同时满足：

1. Git 历史包含实施时采用的精确 upstream SHA。
2. 新增代码不修改 RocksDB submodule。
3. Kiwi 必需的最小 TableProperties API 全部恢复。
4. Factory 满足 `Send + Sync`，callback 不生成共享状态上的 `&mut`。
5. panic/unwind 不跨越 FFI。
6. Collection 和 iterator 不使用 `std::mem::zeroed()` 伪造所有权。
7. bundled/system backend 行为明确且有验证。
8. fork 的必要 fmt、clippy、测试和 CI 通过。
9. Kiwi 更新 revision 后完成 Linux/WSL 集成验证。
10. 旧 SST/custom property 兼容性得到运行时证据。

## 8. PR 和提交策略

建议提交/PR 顺序：

1. `chore: establish Kiwi maintenance baseline`
2. `feat: expose table properties read APIs`
3. `feat: add table properties collector extension`
4. `fix: make table properties collector factory thread-safe`
5. `build: support table properties extensions across backends`
6. `test: cover table properties concurrency and ffi failures`
7. Kiwi 仓库单独提交依赖 revision 和集成测试

每个提交必须能独立解释行为变化，并在计划规定的环境中通过相应最小验证。大范围失败时回退当前语义提交，不在后续提交中掩盖未解决问题。

## 9. 当前 PR #4 的处置

PR #4 在新维护线本地建立和验证前保持不动。

获得 push 授权并建立 `kiwi-maintenance` 后：

1. 核对远端 maintenance SHA、submodule 和 checks。
2. 关闭 PR #4，说明其 Base 选择错误，已由独立维护线取代。
3. 保留 `addtableproperties` 作为迁移证据，直到 Kiwi 集成和旧 SST 兼容验证完成。
4. 后续 PR 只以 `kiwi-maintenance` 为 Base。

关闭 PR、创建远端分支、设置保护规则和 push 都属于外部状态变更，必须在执行前获得单独授权。
