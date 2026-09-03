# Grafana integration example

Visualize archived logs in Grafana using the free
[Infinity datasource plugin](https://grafana.com/grafana/plugins/yesoreyeram-infinity-datasource/)
— no custom ObsidianLog plugin needed.

ObsidianLog has no persistent query HTTP API (querying happens via the CLI),
and Infinity reads from a **URL**, not a local file or a command. So this
example bridges the two with a tiny script: export a query to JSON, serve
it over HTTP, point Infinity at that URL.

This is an example, not a production integration — a dedicated Grafana
datasource plugin is a possible future direction, not something this
project ships today.

## Prerequisites

- A running Grafana instance.
- The Infinity datasource plugin installed:
  `grafana-cli plugins install yesoreyeram-infinity-datasource`, or via the
  in-app plugin catalog.
- `obsidianlog` set up and pointed at an archive with some data in it (see
  the [Quickstart](https://github.com/emmaglorypraise/ObsidianLog#try-it)).

## Run it

```sh
./export-and-serve.sh [path-to-config.toml] [port]   # port defaults to 8787
```

This writes the current query results to `export/logs.json` and serves that
directory over HTTP. Leave it running.

Then in Grafana:

1. **Add the datasource**: either apply `datasource.yaml` via Grafana's
   provisioning mechanism (drop it in `provisioning/datasources/`), or add
   an Infinity datasource manually through the UI.
2. **Import the dashboard**: Dashboards → Import → upload `dashboard.json`,
   selecting your Infinity datasource when prompted. If you changed the
   port above, edit the panel's query URL to match
   (`http://localhost:<port>/logs.json`).

You should see a table of your archived logs — timestamp, service, level,
host, and message (pulled from the original event's `msg`/`message` field).

## Keeping it fresh

`export-and-serve.sh` is one-shot: it snapshots the current query results
once, then serves that snapshot until you stop it. For continuous refresh,
wrap it in `watch -n 60 ./export-and-serve.sh`, a cron job, or a systemd
timer — none of that is included here, to keep this an example rather than
infrastructure to maintain.

## Query filters

Edit the `obsidianlog query` invocation in `export-and-serve.sh` to add any
of `--service`, `--level`, `--host`, `--keyword`, `--from`/`--to` — see the
[CLI reference](https://github.com/emmaglorypraise/ObsidianLog/blob/main/docs-site/cli/query.mdx).
