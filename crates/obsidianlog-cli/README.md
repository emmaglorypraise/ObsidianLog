# obsidianlog

The command-line interface and published binary for **ObsidianLog** — long-term,
tamper-evident operational log archival on [Sia](https://sia.tech).

The crate is `obsidianlog-cli`; it installs a binary named `obsidianlog`
(`init`, `serve`, `query`, `verify`). It builds on the workspace crates
[`obsidianlog-core`](https://crates.io/crates/obsidianlog-core) (shared types and
the `StorageBackend` trait),
[`obsidianlog-store`](https://crates.io/crates/obsidianlog-store) (storage
pipeline) and
[`obsidianlog-ingest`](https://crates.io/crates/obsidianlog-ingest) (HTTP ingest
service).

```sh
cargo install obsidianlog-cli
obsidianlog init
```

See the [project README](https://github.com/emmaglorypraise/ObsidianLog) for the
full overview, architecture, and usage.

## License

[MIT](https://github.com/emmaglorypraise/ObsidianLog/blob/main/LICENSE)
