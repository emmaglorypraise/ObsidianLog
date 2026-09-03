# SIEM export example

`obsidianlog query --format raw` emits NDJSON — one JSON object per line,
the original ingested event, unmodified. Mainstream SIEM tooling ingests
this natively, with no custom parsing: this directory has two example
configs showing it flowing into Filebeat (→ Elasticsearch/Logstash-backed
SIEMs) and Splunk.

This is an example, not a shipped integration — see the configs' own
comments for what to change before using them for real (endpoints, index
names, credentials).

## Export

```sh
obsidianlog query --format raw --from 24h > /var/log/obsidianlog/archive.ndjson
```

Run this on a schedule (cron, a systemd timer) for ongoing export — a
one-off `query` run is a snapshot of what matched at that moment, not a live
tail of new archives as they land.

## Filebeat → Elasticsearch/Logstash

See [`filebeat.yml`](./filebeat.yml). A `filestream` input tails the
exported file, parses each line as NDJSON, and forwards it — no custom
Filebeat processors needed since the shape is already flat JSON per line.

```sh
filebeat -e -c filebeat.yml
```

## Splunk

See [`splunk-inputs.conf`](./splunk-inputs.conf). A `monitor://` stanza with
`INDEXED_EXTRACTIONS = json` tells Splunk to parse each line as a JSON
event directly, same reasoning as the Filebeat config.

Drop it into `$SPLUNK_HOME/etc/apps/<your_app>/local/inputs.conf` and
restart the forwarder/instance.

## Why this works with zero custom integration code

ObsidianLog's `--format raw` output is exactly the log event as it was
originally ingested — SIEM tools that already know how to parse JSON logs
(which is most of them) need nothing ObsidianLog-specific to consume it.
The interesting property being demonstrated here isn't the export format
(that's just JSON) — it's that what you're exporting was tamper-evident and
hash-chained the whole time it was archived (`obsidianlog verify`), unlike
logs that only ever lived in a mutable hot-tier index.
