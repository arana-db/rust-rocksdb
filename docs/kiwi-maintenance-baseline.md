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
