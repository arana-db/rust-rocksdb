# EventListener 安全加固实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 在当前 Arana 维护线上修复 EventListener 的生命周期、native ownership 和 panic 边界，并记录最后同步的 `zaidoon1/rust-rocksdb` commit。

**架构：** 保留现有完整 EventListener 事件集合和 C++ extension，只收紧 Rust 安全接口。注册入口要求 `'static`，callback-only status 改为借用，公开但字段不透明的低层 handle 通过 RAII 管理转移前所有权，所有用户回调和析构使用统一 fail-fast panic helper。

**技术栈：** Rust 1.91、C/C++ FFI、RocksDB 11.1.2、rustdoc compile-fail、WSL/Linux、Cargo integration tests。

---

### 任务 1：建立生命周期编译门禁

**文件：**
- 修改：`src/db_options.rs:1788-1791`
- 修改：`src/event_listener.rs:508-525,618-637,639-798`

- [x] **步骤 1：添加非 `'static` listener 的 compile-fail 文档测试**

在 `Options::add_event_listener` 文档中加入可独立编译的 `compile_fail,E0597`
示例。示例定义借用局部 `AtomicUsize` 的 listener，并调用
`options.add_event_listener(listener)`；当前实现会错误编译成功，因此 doctest 应
红灯。

- [x] **步骤 2：添加精确签名和 `MutableStatus` 逃逸文档测试**

先用正向 doctest 按精确签名实现
`on_background_error(..., status: &MutableStatus)`，锁定公开 trait 的参数类型；再
增加 `compile_fail,E0521` 示例，尝试把真实 callback 参数保存到 `'static` 位置，
验证引用不能逃逸 callback 生命周期。旧按值签名由前一个正向测试识别；不能声称
它会让同一个 E0521 逃逸示例编译成功。

- [x] **步骤 3：运行 doctest 验证预期红灯**

运行：

```bash
CARGO_TARGET_DIR=/tmp/rust-rocksdb-event-listener-target cargo test --doc
```

预期：在旧按值 API 上，正向精确签名 doctest 因 trait 方法参数类型不匹配而失败；
接口改为借用后，正向示例通过，逃逸示例以 E0521 编译失败并被 rustdoc 视为通过。
其他 doctest 不能失败。

- [x] **步骤 4：写入最小生命周期修复**

将注册入口和内部构造器约束为：

```rust
pub fn add_event_listener<L>(&mut self, listener: L)
where
    L: EventListener + 'static,
```

```rust
pub fn new_event_listener<E>(listener: E) -> DBEventListener
where
    E: EventListener + 'static,
```

把 trait 方法改为 callback-scoped 借用：

```rust
fn on_background_error(&self, _: DBBackgroundErrorReason, _: &MutableStatus) {}
```

trampoline 构造局部 `MutableStatus` 后以 `&status` 调用用户实现。不得改变
`MutableStatus::reset/result/severity` 行为。

- [x] **步骤 5：验证生命周期绿灯**

运行：

```bash
CARGO_TARGET_DIR=/tmp/rust-rocksdb-event-listener-target cargo test --doc
CARGO_TARGET_DIR=/tmp/rust-rocksdb-event-listener-target cargo test --test test_event_listener --features multi-threaded-cf
```

预期：compile-fail 示例和现有 5 个 EventListener 集成测试全部通过。

- [x] **步骤 6：提交生命周期合同**

```bash
git add src/db_options.rs src/event_listener.rs
git commit -m "fix(event-listener): enforce callback lifetimes"
```

### 任务 2：为 native listener handle 建立 RAII

**文件：**
- 修改：`src/event_listener.rs:769-798`
- 修改：`src/db_options.rs:1788-1791`
- 修改测试：`src/event_listener.rs` 内部测试模块
- 修改测试：`tests/test_event_listener.rs`

- [x] **步骤 1：编写未注册 handle 析构测试**

在 `src/event_listener.rs` 的 `#[cfg(test)]` 模块定义带 `Arc<AtomicUsize>` 的
listener，直接调用公开的低层 constructor 后 drop handle，断言 listener
析构计数为 `1`。当前 `DBEventListener` 没有 `Drop`，测试应失败为 `0`。

- [x] **步骤 2：编写 Options 到 DB 的所有权转移测试**

在 `tests/test_event_listener.rs` 增加真实 DB 测试：注册 listener、打开 DB、drop
Options、执行 put/flush，确认 callback 仍触发；drop DB 后 listener 析构计数恰好
为 `1`。

- [x] **步骤 3：运行定向测试验证红灯**

```bash
CARGO_TARGET_DIR=/tmp/rust-rocksdb-event-listener-target cargo test event_listener_handle -- --nocapture
CARGO_TARGET_DIR=/tmp/rust-rocksdb-event-listener-target cargo test --test test_event_listener listener_ownership -- --nocapture
```

预期：未注册 handle 的 Drop 计数失败；不得接受数据库打开失败作为红灯。

- [x] **步骤 4：实现公开低层 RAII handle**

保留公开但字段不透明的低层 handle：

```rust
pub struct DBEventListener {
    inner: *mut ffi::rust_rocksdb_eventlistener_t,
    owned: bool,
}
```

要求：

- constructor 返回 `owned: true`。
- `into_raw(mut self)` 在返回指针前设置 `owned = false`。
- `Drop` 仅在 `owned && !inner.is_null()` 时调用
  `rust_rocksdb_eventlistener_destroy`。
- 不实现 `Send` 或 `Sync`。
- `Options::add_event_listener` 只把 `handle.into_raw()` 交给 C++。
- `DBEventListener` 和 `new_event_listener` 保持 public symbol，避免删除既有低层
  API；RAII 只修复 ownership，不收窄可见性。

- [x] **步骤 5：验证 ownership 绿灯**

运行任务 2 的两个定向命令，并执行完整 EventListener 测试。预期未注册、已注册和
DB 最终销毁三条路径都只析构一次。

- [x] **步骤 6：提交 ownership 修复**

```bash
git add src/event_listener.rs src/db_options.rs tests/test_event_listener.rs
git commit -m "fix(event-listener): manage native listener ownership"
```

### 任务 3：统一隔离 callback 和 destructor panic

**文件：**
- 修改：`src/event_listener.rs:639-767`
- 修改测试：`tests/test_event_listener.rs`

- [x] **步骤 1：编写 flush callback panic 子进程测试**

参照 `tests/test_table_properties_collector_factory.rs` 的 child-process 模式，定义
固定入口标记和模式环境变量。子进程注册一个在 `on_flush_begin` panic 的 listener
并触发真实 flush；父进程断言非成功退出、入口标记存在，且 stderr 包含：

```text
rust-rocksdb: event listener on_flush_begin callback panicked
```

- [x] **步骤 2：编写 destructor panic 子进程测试**

子进程注册一个 `Drop` 会 panic 的 listener，打开并销毁 DB；父进程断言 stderr
包含：

```text
rust-rocksdb: event listener destructor callback panicked
```

- [x] **步骤 3：运行子进程测试验证红灯**

```bash
CARGO_TARGET_DIR=/tmp/rust-rocksdb-event-listener-target cargo test --test test_event_listener event_listener_panic -- --nocapture
```

预期：当前实现没有固定诊断，因此父测试失败。子进程能到达标记入口。

- [x] **步骤 4：实现统一 fail-fast helper**

在 `src/event_listener.rs` 增加：

```rust
fn abort_on_panic(callback: &str, f: impl FnOnce()) {
    if catch_unwind(AssertUnwindSafe(f)).is_err() {
        eprintln!("rust-rocksdb: event listener {callback} callback panicked");
        process::abort();
    }
}
```

所有 EventListener trampolines 和 destructor 都必须通过该 helper 调用用户代码。
unsafe 解引用放在最小作用域，并添加对应 `SAFETY` 注释。不得只包装 flush 路径。

- [x] **步骤 5：验证 panic 绿灯和行为回归**

运行：

```bash
CARGO_TARGET_DIR=/tmp/rust-rocksdb-event-listener-target cargo test --test test_event_listener event_listener_panic -- --nocapture
CARGO_TARGET_DIR=/tmp/rust-rocksdb-event-listener-target cargo test --test test_event_listener --features multi-threaded-cf -- --nocapture
```

预期：两个子进程合同和全部真实 EventListener 测试通过。

- [x] **步骤 6：提交 panic 边界**

```bash
git add src/event_listener.rs tests/test_event_listener.rs
git commit -m "fix(event-listener): fail fast on callback panics"
```

### 任务 4：记录 upstream 同步基线和公开变更

**文件：**
- 修改：`docs/kiwi-maintenance-baseline.md`
- 修改：`CHANGELOG.md`

- [x] **步骤 1：更新同步状态字段**

在维护文档写明：

```text
Last synchronized upstream commit:
a27cb5bdbdb74550835ed5820ad02817c9a8c457
Verified: 2026-07-23
Maintenance branch: master
```

保留 rust-rocksdb、sys、RocksDB 和 Snappy 版本/SHA。

- [x] **步骤 2：加入可复制的未来同步流程**

文档包含：

```bash
git fetch actual-upstream master
git merge-base --is-ancestor <last-synced-sha> master
git rev-list --left-right --count <last-synced-sha>...master
```

说明核验快照为 upstream-only `0`、Arana-only `22`；数字会随 Arana 新提交增加，
真正门禁是 upstream-only 必须为 `0`。实际同步前重新获取
`actual-upstream/master`，更新 commit、submodule 和版本记录。

- [x] **步骤 3：修正文档中的维护分支语义**

把“`kiwi-maintenance` 是 base branch”改为当前真实的 `master` Arana 维护线；保留
`actual-upstream` push URL 必须为 `DISABLED` 的规则。

- [x] **步骤 4：更新 Changelog**

在顶部 `Unreleased` 段记录 breaking change；`0.51.0` 已在 `db45d89` 发布，
不得回写：

- `on_background_error` 从 `status: MutableStatus` 改为
  `status: &MutableStatus`，下游 trait impl 必须修改签名。
- EventListener registration requires `'static`。
- callback/destructor panic 使用固定诊断并 fail-fast。
- public `DBEventListener`/`new_event_listener` 保持兼容，native listener
  constructor ownership 不再泄漏。

- [x] **步骤 5：验证文档并提交**

```bash
rg -n 'a27cb5bdbdb74550835ed5820ad02817c9a8c457|Last synchronized|actual-upstream|upstream-only|Arana-only' docs/kiwi-maintenance-baseline.md
git diff --check
git add docs/kiwi-maintenance-baseline.md CHANGELOG.md
git commit -m "docs: record rust-rocksdb upstream sync point"
```

Review correction 执行记录（2026-07-23）：

- 确认 `on_background_error` 的按值到借用变更是公开 breaking change，并把记录从
  已发布的 `0.51.0` 移到 `Unreleased`。
- 将 `MutableStatus` 门禁纠正为“正向精确签名 doctest + E0521 逃逸
  compile-fail”，删除“旧按值会让同一个逃逸示例编译成功”的错误完成记录。
- 保留公开的 `DBEventListener` 和 `new_event_listener`；RAII 仅管理转移前所有权，
  不通过降低可见性制造额外兼容性破坏。

### 任务 5：最终质量门禁和交付

**文件：**
- 不新增生产文件

- [x] **步骤 1：格式与文档**

```bash
cargo fmt --all -- --check
cargo test --doc
cargo rustdoc -- -D warnings
git diff --check origin/master..HEAD
```

- [x] **步骤 2：定向和 feature 测试**

```bash
CARGO_TARGET_DIR=/tmp/rust-rocksdb-event-listener-target cargo test --test test_event_listener -- --nocapture
CARGO_TARGET_DIR=/tmp/rust-rocksdb-event-listener-target cargo test --test test_event_listener --features multi-threaded-cf -- --nocapture
```

- [x] **步骤 3：Lint 与默认测试**

按 `.github/workflows/rust.yml` 的实际命令运行 Clippy 和默认测试；不得机械使用
`--all-features`，因为 `coroutines` 依赖 Folly/liburing。

- [x] **步骤 4：system RocksDB 验证**

若本机存在 PR #8 缓存的 pinned system RocksDB 11.1.2，则使用相同
`ROCKSDB_LIB_DIR`、`ROCKSDB_INCLUDE_DIR` 和动态链接设置运行 EventListener 测试；
否则记录环境缺口，并以 GitHub `Linux (system RocksDB)` 为最终远端门禁。

执行记录（2026-07-23）：本机及 review worktree 中未找到
`target/system-rocksdb/librocksdb.so`，因此未伪造或临时重建该缓存；system backend
留给 PR 的 GitHub `Linux (system RocksDB)` job 验证。

- [x] **步骤 5：规格与代码质量双重审查**

逐项核对设计目标、API 兼容、unsafe 依据、panic 策略、测试变异强度、同步文档和
无关 Diff。P0/P1 未清零前不得 push。

- [ ] **步骤 6：实时刷新 Base 并创建替代 PR**

重新获取 `arana-db/master`。若 Base 前移，rebase 后重跑受影响门禁。然后 push
`codex/event-listener-safety`，创建 ready-for-review PR。PR 正文说明旧 PR #1 已被
当前基线功能覆盖，新 PR 只做安全加固和同步记录；不关闭 PR #1。
