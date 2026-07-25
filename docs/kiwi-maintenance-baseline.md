# Kiwi rust-rocksdb Maintenance Baseline

The Arana `master` branch is the development line from which maintenance tags
are produced. Kiwi consumes a reviewed, immutable maintenance tag rather than
the moving branch.

## Source baseline

- Source repository: `https://github.com/arana-db/rust-rocksdb.git`
- Last synchronized upstream commit: `a27cb5bdbdb74550835ed5820ad02817c9a8c457`
- Latest upstream head observed: `938a365d9497dcd43204eff1cf8a76c09745e541`
- Final verification: `2026-07-25`
- Maintenance branch: `master`
- Arana `master` commit at verification: `971c792f3d6312204a5e162bd60d4d3c84a9e8a8`
- Maintenance tag: `v0.51.0-arana.1`
- Annotated tag object: `4a9d2fec47ec467b00a9701f9db14eaffeca806a`
- Tag target commit: `971c792f3d6312204a5e162bd60d4d3c84a9e8a8`
- rust-rocksdb: `0.51.0`
- rust-librocksdb-sys: `0.47.1+11.1.2`
- MSRV: Rust `1.91`
- RocksDB submodule: `3b446089141659fad25328c5ea3e7ed283df46e4`
- Snappy submodule: `6af9287fbdb913f0794d0148c6aa43b58e63c8e3`

At the upstream synchronization verification on `2026-07-23`, comparing the
synchronized commit with Arana `master` produced `0` upstream-only commits and
`22` Arana-only commits. Here,
"synchronized" means that `master` contains the recorded upstream commit; it
does not mean that both repositories have identical trees. The Arana-only
count is expected to grow as maintenance patches are added. The synchronization
point containment gate is that the recorded commit has `0` upstream-only
commits when compared with `master`.

At that `2026-07-23` upstream comparison, `actual-upstream/master` had advanced
to `938a365d9497dcd43204eff1cf8a76c09745e541`. Comparing that live upstream head
with Arana `master` produced `2` upstream-only commits and `22` Arana-only
commits. Arana `master` therefore contains the recorded synchronization point
but is not fully current with the latest upstream. The two pending upstream
commits are:

- `df19be8f9489545dd6eca917f35d69c19a3b8e91` — `perf: reduce wrapper
  allocations and FFI overhead`
- `938a365d9497dcd43204eff1cf8a76c09745e541` — `bench: cover
  allocation-sensitive RocksDB paths`

They are inputs for the next synchronization and are not imported by this
change. The fully-current gate is that comparing the freshly fetched
`actual-upstream/master` with `master` produces `0` upstream-only commits.

Reproduce the ancestry check from an up-to-date `master` checkout with:

```bash
git fetch actual-upstream master
git merge-base --is-ancestor <sha> master
git rev-list --left-right --count <sha>...master
```

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

1. `master` is the base branch for Arana-specific changes.
2. Before the next synchronization, fetch `actual-upstream/master` again and
   verify ancestry before replaying or adapting Arana patches.
3. Each Arana capability is introduced in a focused, tested commit.
4. RocksDB submodule sources are not modified by Arana extensions.
5. Kiwi pins the reviewed Arana maintenance tag; `Cargo.lock` records the exact
   resolved commit SHA so dependency resolution remains auditable.
6. After synchronization, update the recorded upstream commit, submodule SHAs,
   and crate versions in this document.
