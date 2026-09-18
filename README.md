# ObsidianLog

[![CI](https://github.com/emmaglorypraise/ObsidianLog/actions/workflows/ci.yml/badge.svg)](https://github.com/emmaglorypraise/ObsidianLog/actions/workflows/ci.yml)
[![Demo](https://github.com/emmaglorypraise/ObsidianLog/actions/workflows/demo.yml/badge.svg)](https://github.com/emmaglorypraise/ObsidianLog/actions/workflows/demo.yml)
[![Docs](https://img.shields.io/badge/docs-obsidianlog.mintlify.site-5B4FE9)](https://obsidianlog.mintlify.site/)

> Long-term, tamper-evident operational log archival on [Sia](https://sia.tech).

ObsidianLog is a cold-tier destination for operational logs. It sits beside
your hot observability stack, such as Grafana, Datadog, or ELK, and archives
logs encrypted, compressed, and hash-chained for later retrieval and
verification. You control the encryption keys.

It is for teams and developers who need logs to remain private, retrievable,
and tamper-evident after they leave their primary monitoring system.

> **Trying the beta?** Start at the [Beta Testing page](https://obsidianlog.mintlify.site/beta).
> It explains what to test, what a completed test looks like, and how to give
> feedback.

## Get started

Download the archive for your platform from the
[Releases page](https://github.com/emmaglorypraise/ObsidianLog/releases).
The official `obsidianlog` binary includes both the local and Sia backends.
No source build is required.

For complete macOS, Windows, and Linux installation instructions, including
Gatekeeper and SmartScreen guidance, see the
[Quickstart](https://obsidianlog.mintlify.site/get-started/quickstart).

On macOS or Linux, after extracting the archive in a terminal:

```sh
chmod +x obsidianlog obsidianlog-ingest
./obsidianlog init
```

`obsidianlog` is a terminal application, not a GUI application. Run it with a
subcommand such as `init`. Double-clicking it only prints help and exits.

## Five-minute local check

Choose the **local** backend in `init`. It needs no Sia account and keeps the
sample data on your machine. Leave the server running in one terminal:

```sh
./obsidianlog init
./obsidianlog serve
```

In a second terminal in the same folder, send and verify one sample log:

```sh
curl -s -X POST http://localhost:7080/ingest \
  -H 'content-type: application/json' \
  -d '[{"timestamp":"2026-09-18T10:00:00Z","service":"api","level":"info","msg":"hello"}]'

./obsidianlog query --service api --level info --from 24h --format human
./obsidianlog verify
```

For step-by-step platform guidance, see the
[local-backend tutorial](https://obsidianlog.mintlify.site/tutorials/local-backend).

## Sia-backed archival

Choose **sia** during `init` to archive through a Sia indexer. The default
hosted option uses [sia.storage](https://sia.storage), where you approve
ObsidianLog through your own account. You can also supply a self-hosted or
third-party indexer. The first Sia write may take several minutes, so allow
15–20 minutes for the full test. Use sample data only while evaluating it.

Follow the [Sia backend tutorial](https://obsidianlog.mintlify.site/tutorials/sia-backend)
for the complete flow and current hosted-service considerations.

## Documentation

- [Quickstart](https://obsidianlog.mintlify.site/get-started/quickstart): install and run the full local loop.
- [Choosing a backend](https://obsidianlog.mintlify.site/storage-backends/choosing-a-backend): local, hosted Sia Storage, or your own indexer.
- [Send logs with Vector](https://obsidianlog.mintlify.site/deployment/sending-logs-with-vector): connect a real log shipper.
- [Docker Compose quickstart](https://obsidianlog.mintlify.site/deployment/docker-compose-quickstart): run the local backend in a container.
- [CLI reference](https://obsidianlog.mintlify.site/cli/overview): `init`, `serve`, `query`, and `verify`.
- [Architecture and security](https://obsidianlog.mintlify.site/architecture-and-security/pipeline-architecture): pipeline, threat model, and design decisions.
- [Building from source](https://obsidianlog.mintlify.site/get-started/building-from-source) and [testing against a live Sia indexer](https://obsidianlog.mintlify.site/project/testing-against-sia): developer setup and optional live-Sia integration testing.
- [Grant progress tracker](docs/grant/PROGRESS.md): milestones, delivery status, and current success criteria.

## Try the live demo

Fork this repository and run the
[Demo workflow](.github/workflows/demo.yml) from the Actions tab. It runs the
real local pipeline, `init`, `serve`, ingest, `query`, and `verify`, with no
Sia account or secrets required.

## Feedback, contributing, and security

- [Give beta feedback](https://github.com/emmaglorypraise/ObsidianLog/issues/new?template=feedback.yml), [report a bug](https://github.com/emmaglorypraise/ObsidianLog/issues/new?template=bug_report.yml), or [ask a question](https://github.com/emmaglorypraise/ObsidianLog/discussions).
- Read [CONTRIBUTING.md](CONTRIBUTING.md) before proposing a change.
- See [SECURITY.md](SECURITY.md) for vulnerability reporting and the security model.

## License

[MIT](LICENSE) © Glory Praise Emmanuel.
