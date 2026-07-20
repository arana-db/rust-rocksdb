# Kiwi 维护基线

本文档记录 Kiwi 专用 RocksDB 维护线的基线信息，作为后续移植、CI 复核和 Cargo pin 审计的统一来源。

## 实际 upstream

- **URL**: `https://github.com/zaidoon1/rust-rocksdb.git`
- **Remote name**: `actual-upstream`
- **Push URL**: `DISABLED`（禁止向 upstream 推送任何内容）
- **HEAD SHA**: `a27cb5bdbdb74550835ed5820ad02817c9a8c457`
- **记录时间**: 2026-07-20

## Rust package

### rust-rocksdb

- **name**: `rust-rocksdb`
- **version**: `0.51.0`
- **edition**: `2024`
- **MSRV**: `1.91`

### librocksdb-sys

- **name**: `librocksdb-sys`
- **version**: `0.47.1+11.1.2`
- **edition**: `2024`
- **MSRV**: `1.91`

## Submodule

### RocksDB

- **path**: `librocksdb-sys/rocksdb`
- **SHA**: `3b446089141659fad25328c5ea3e7ed283df46e4`
- **tag**: `v11.1.2`
- **URL**: `https://github.com/facebook/rocksdb.git`

### Snappy

- **path**: `librocksdb-sys/snappy`
- **SHA**: `6af9287fbdb913f0794d0148c6aa43b58e63c8e3`
- **tag**: `1.2.2`
- **URL**: `https://github.com/google/snappy.git`

## Arana 旧定制分支

- **分支**: `addtableproperties`
- **HEAD SHA**: `f7abb18c64fac810f3c4736aef833c340396449b`
- **状态**: 仅作为移植参考，不作为新基线的起点
- **原因**: build.rs、FFI 布局、c-api-extensions 框架已在新 upstream 重构，旧分支无法直接 merge

## Arana 旧 Cargo pin

- **Kiwi 当前依赖**:
  ```toml
  rocksdb = { git = "https://github.com/arana-db/rust-rocksdb.git", rev = "f7abb18c64fac810f3c4736aef833c340396449b", features = ["multi-threaded-cf"] }
  ```

## 远端状态边界

**严格禁止**：

- 向 `zaidoon1/rust-rocksdb` 推送任何分支、tag、PR、issue
- 修改 upstream 的任何远端状态
- 将本维护线的代码回馈给 upstream

**允许**：

- 从 `actual-upstream` fetch 代码
- 在 Arana 仓库内创建、推送、合并分支
- 向 Arana 仓库提交 PR 和 issue

## 阶段一验证

### 验证目标

确认新维护分支 `codex/kiwi-table-properties` 的 HEAD 等于 upstream SHA，工作树干净，submodule 完全匹配。

### 验证命令

```bash
# 检查 HEAD SHA
git rev-parse HEAD

# 检查 upstream 是否为 HEAD 的祖先
git merge-base --is-ancestor a27cb5bdbdb74550835ed5820ad02817c9a8c457 HEAD

# 检查工作树状态
git status --short

# 检查 submodule 状态
git submodule status
```

### 验证环境

- **OS**: Windows 11 24H2 (Git Bash)
- **Git**: 2.x
- **执行时间**: 2026-07-20

### 验证结果

✅ **全部通过**

- HEAD SHA: `a27cb5bdbdb74550835ed5820ad02817c9a8c457`
- upstream 祖先检查: 通过
- 工作树: 干净
- submodule SHA: 完全匹配，无 `+`、`-` 或冲突标记

## 下一步

阶段一完成后，进入阶段二：在新基线上重新移植 Kiwi 所需的 TableProperties API。

---

**维护人**: Kiwi 团队  
**最后更新**: 2026-07-20
