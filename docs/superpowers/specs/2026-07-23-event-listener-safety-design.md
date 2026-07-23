# EventListener 安全边界设计

## 背景

`arana-db/rust-rocksdb` 当前维护线已经基于
`zaidoon1/rust-rocksdb@a27cb5bdbdb74550835ed5820ad02817c9a8c457`
包含完整的 Rust `EventListener` 类型和测试，并通过 Arana 的本地 C API
extension 将这些回调接入 bundled 与 system RocksDB。旧 PR #1 只实现两个
flush 回调，其余回调是空 stub，且基于已经废弃的历史维护线，因此不移植旧
提交。

本设计在当前 `master` 上加固已经存在的公开能力，不重新定义事件集合，也不
修改 RocksDB submodule。

## 目标

1. 阻止借用非静态数据的 listener 被交给长期存活的 C++ `shared_ptr`。
2. 让 background-error 的可变 native status 只能在 callback 期间使用。
3. 为尚未转移给 RocksDB 的 native listener wrapper 建立 RAII 所有权。
4. 保证用户 callback 或 listener 析构 panic 不会尝试跨越 C ABI unwind。
5. 在长期维护文档中记录最后一次同步的 `zaidoon1` commit 和未来同步命令。

## 非目标

- 不复制 PR #1 的旧 `rocksdb_eventlistener_*` 接口或 TODO stub。
- 不把所有 event info 改成拥有数据的 snapshot。
- 不改变现有事件种类、正常 callback 顺序或属性访问结果。
- 不修改 `zaidoon1/rust-rocksdb`，也不向该仓库发送 branch、PR、Issue 或评论。
- 不修改 RocksDB 或 Snappy submodule revision。

## 生命周期合同

`Options::add_event_listener` 和公开的低层构造函数都要求其 listener 类型满足
`EventListener + 'static`。listener 会被 `Box` 固定地址并交给 C++
`shared_ptr<EventListener>`，因此它不能借用调用栈上的临时数据。使用 `Arc` 等
拥有型共享状态的 listener 仍然兼容。

`MutableStatus` 不再按值传给 `on_background_error`，改为 callback-scoped 的
`&MutableStatus`。wrapper 仍可在 callback 内调用 `result()`、`severity()` 和
`reset()`，但安全 Rust 不能把它移动到 callback 外继续访问 native `Status`。

其他 job info 类型已经通过借用传递，内部指针私有、类型不可 `Copy`/`Clone`，
本轮保持现有接口；访问字符串时继续立即复制为 `Vec<u8>`。

## 所有权模型

公开但字段不透明的 `DBEventListener` 唯一拥有
`rust_rocksdb_eventlistener_t*`。为保留现有低层 API 兼容性，构造器保持：

```rust
pub fn new_event_listener<E>(listener: E) -> DBEventListener
where
    E: EventListener + 'static,
```

所有权规则如下：

- 构造成功后，未注册的 handle 在 `Drop` 中调用 native destroy。
- `Options::add_event_listener` 通过 `into_raw()` 把所有权转移给 RocksDB。
- 转移后 Rust handle 不再析构该指针；C++ `shared_ptr` 在最后一个 owner 销毁时
  调用 native listener 析构，并最终释放 `Box<L>`。
- 底层 `DBEventListener` 和 `new_event_listener` 保持公开；handle 字段仍然私有，
  不扩大可直接操作 raw pointer 的接口。
- 不为 handle 添加没有证明需要的 `unsafe impl Send/Sync`。

## Panic 策略

所有 Rust listener callbacks 和 destructor 通过一个统一 helper 调用用户代码：

1. `catch_unwind(AssertUnwindSafe(...))` 捕获 panic。
2. 输出固定、可测试的诊断：
   `rust-rocksdb: event listener <callback> callback panicked`。
3. 调用 `process::abort()`。

不静默吞掉 panic。background-error 和 recovery callback 可以改变 RocksDB 状态，
而用户 listener 在 panic 后也可能处于不一致状态；确定性 fail-fast 比继续运行
更安全，并与仓库现有 FFI callback 策略一致。

## 测试设计

### 编译期合同

使用两类互补的 rustdoc 门禁验证：

- 借用局部变量的 listener 不能传给 `Options::add_event_listener`。
- 正向 doctest 用精确函数签名实现
  `on_background_error(..., status: &MutableStatus)`，直接锁定公开 trait 签名。
- `compile_fail,E0521` 示例尝试把真实 callback 参数 `&MutableStatus` 保存到
  `'static` 位置，证明它不能逃逸 callback 生命周期。

精确签名 doctest 负责识别旧的按值 API；E0521 示例负责验证改为借用后的逃逸
边界。不能把两者混为“旧按值实现会让同一个逃逸示例编译成功”。

### 所有权

- 直接构造并丢弃未注册 handle 时，listener `Drop` 恰好执行一次。
- `Options` 被销毁但 DB 仍存活时 callback 仍能执行。
- DB 最终销毁后 listener `Drop` 恰好执行一次。

### Panic 边界

使用子进程分别触发 flush callback panic 和 listener destructor panic，断言：

- 子进程非成功退出。
- 已到达指定测试入口。
- stderr 包含精确 event-listener 诊断，而不是依赖泛化的 ABI panic 文本。

### 行为回归

保留并运行现有真实 flush、stall、background-error 和 recovery 测试；同时运行
default 与 `multi-threaded-cf` 配置。最终执行 rustfmt、rustdoc、doctest、Clippy
和项目 CI 对应测试。

## 同步基线文档

更新 `docs/kiwi-maintenance-baseline.md`，明确：

- Source repository：`https://github.com/zaidoon1/rust-rocksdb.git`
- Last synchronized upstream commit：
  `a27cb5bdbdb74550835ed5820ad02817c9a8c457`
- 核验日期：`2026-07-23`
- 当前 Arana 维护线：`master`
- 同步关系：upstream-only `0`，Arana-only `22`（以该核验快照为准）
- `merge-base --is-ancestor` 与 `rev-list --left-right --count` 核验命令
- 下次同步时先 fetch-only 获取 `actual-upstream/master`，再重放 Arana 增量

文档必须说明“已同步”表示完整包含该 upstream commit，不表示两个仓库 tree
逐字节相同。

## 交付边界

新 PR 从当前 `arana-db/master` 派生，替代旧 PR #1 的交付意图。新 PR 创建后保留
工作树用于 review 迭代；旧 PR #1 由用户关闭，本任务不执行关闭操作。
