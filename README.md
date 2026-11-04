# DASRU

DOOM: The Dark Ages SpeedRun Utility. The first slice is a read-only Windows x64 identity probe.

```text
dasru probe
dasru probe --json
```

The probe treats the process name as a candidate filter, hashes the executable on disk, and compares that hash with `data/builds.toml`. The initial registry is empty, so no executable is reported as a known build yet. Build compatibility does not establish run integrity, and neither the local registry nor its report is a tamper-proof attestation. Linux process discovery, Steam metadata discovery, and game-memory access are not implemented.

The upstream [DESRU repository is marked GPL-3.0](https://github.com/bowsr/DESRU). No DESRU source was copied into this first slice; this checkout does not contain the separate research notes needed to clear their provenance. No project license is selected. Review those notes and any excerpts before publishing under MIT.
