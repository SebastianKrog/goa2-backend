# Maintenance performance baseline

Source `ed1597be`, Rust 1.99.1, release profile, Linux x86_64. Three fresh
benchmark processes and three fresh disposable-database server starts per pack.
[Raw results](../../benchmarks/maintenance-v1-results.json) preserve every sample,
program/binary/content/catalog hashes, hardware, work counts and memory snapshots;
[derived summary](performance-summary.json) records medians and ELF section sizes.
Hardware: Intel Xeon Platinum 8370C @ 2.80 GHz, five affinity CPUs, cgroup quota
four CPUs, kernel 6.18.57. Host load and filesystem/page caches are uncontrolled.
Fresh-process startup does not claim a cold OS cache. Samples were collected
after the build/test workload finished. Three samples are an initial baseline,
not a statistical speedup claim or latency service-level objective.

| Median measurement | Seven heroes | 33 heroes | Unit |
| --- | ---: | ---: | --- |
| Content parse | 113.460 | 962.483 | ms |
| First native catalog preparation | 357.399 | 188.837 | ms |
| Warm catalog guard | 6.020 | 5.960 | ns/call |
| Game creation, repeated | 19.863 | 21.222 | ms/op |
| Native session clone + drop | 6.222 | 6.165 | us/op |
| Checkpoint JSON parse + restore | 23.961 | 23.102 | ms/op |
| Prepare commitment | 15.347 | 17.005 | us/op |
| Fixed planning search | 1.549 | 1.116 | ms/search |
| Fixed search throughput | 645.555 | 896.279 | searches/s |
| 5 ms decision budget | 5.028 | 5.032 | ms |
| Cooperative cancellation response | 401.880 | 476.205 | us |
| Benchmark peak RSS | 152552.000 | 167544.000 | KiB |
| Server ready, fresh process/database | 58.451 | 122.418 | ms |
| First game request | 316.608 | 394.102 | ms |
| Second game request | 42.620 | 42.620 | ms |
| Server RSS after two games | 149252.000 | 163772.000 | KiB |

The same four-hero Quick opening is used for both packs: Arien/Brogan versus
Tigerclaw/Sabina, Forgotten Island, seed 29. Per process: 20 repeated setups,
200 session clone/drop operations, 40 JSON checkpoint parse/restores, 100 prepared
commitments and four planning searches. This is opening-position throughput,
not complete-game throughput. Each checkpoint is 300,429 bytes. Clone cost is
NativeSession cloning/drop, not a claim about every application object's size.
RSS includes allocator/catalog/process state; it does not measure live clone size.

Fixed search uses cap 2, depth 1, max 256 nodes, one worker, adversarial opponent
and material-position evaluation. All repeated calls and fresh processes agree:
24 nodes, four evaluated cells, two completed rows, score zero. Diagnostic
Symbol/CardId strings in the raw output are per-build internal identifiers,
not portable game identities. This baseline is reproducible for future same-work
comparisons; it does not establish the full refactor regression baseline C10-01.
The separate 5 ms decision measurement records latency/fallback, with no strength
claim. All six cancellation samples observe cancellation after a requested 10 ms
delay; response is measured from the actual flag signal, not from thread creation.
Cooperative checks do not guarantee hard instruction preemption.

## Catalog and footprint decision

The release server is 23,363,784 bytes (ELF text 21,222,058, data 167,984, bss
3,496); the benchmark is 16,666,376 bytes (text 15,682,569, data 70,272, bss
3,520), measured by GNU `size`. These include more than embedded catalogs.
The raw report inventories all eight catalog raw byte sizes and their unchanged
manifest hashes. First preparation includes JSON parsing/import/compilation;
we did not attribute every millisecond or text-section byte to an individual
catalog. The warm guard is near timer/loop overhead and is not meaningful at
nanosecond precision. Retain the immutable process-wide OnceLock caches and
existing catalog placement; neither repeated parsing nor a measured size target
justifies an optimization here. No catalog bytes, mutable continuation state,
physical aliases or rules were changed. Full parity/restore gates still precede
representation changes.

## Reproduction

From `rust/`, build the pinned release artifacts:

```sh
cargo build --locked --release --workspace --bins --example maintenance_bench
```

From the repository root (use a new output path):

```sh
python3 rust/tools/maintenance_measurements.py --benchmark-binary /absolute/path/release/examples/maintenance_bench --server-binary /absolute/path/release/goa2-game-server --content rust/content/smoke-game.json --content /absolute/path/live-game-content.json --repetitions 3 --output /absolute/path/new-measurements.json
```

The full pack's recorded hash is an input identity, not an established public
acquisition route or rights grant. Results generated using another binary,
compiler, hardware or workload require their own report; do not overwrite this
baseline or compare deadline-limited results as equal work.
