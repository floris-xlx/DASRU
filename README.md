# DASRU

DOOM: The Dark Ages SpeedRun Utility. The first slice is a read-only Windows x64 identity probe.

```text
dasru probe
dasru probe --json
dasru inspect --exe "C:\path\to\DOOMTheDarkAges.exe"
dasru inspect --exe "C:\path\to\DOOMTheDarkAges.exe" --json
```

`inspect --exe` hashes and reads version metadata from an executable without looking for a running process. Its report uses `PROCESS_NOT_EVALUATED` semantics (`process_status: NOT_EVALUATED`), with an empty process list. `probe` discovers a process by executable name, retains its Windows process handle while it hashes the executable, and reports the live process identity.

JSON reports use `schema_version: 1`; `observed_at_unix_ms` is the UTC Unix epoch time in milliseconds. `status` reports the overall executable/process result. `executable_status` and `steam_status` keep executable recognition separate from Steam metadata. The Steam manifest is checked for App ID, `installdir`, and Build ID format; its resolved install directory must contain the inspected executable. A registered Steam Build ID is compared separately, so a mismatch does not erase a known executable hash. Field provenance and structured errors identify the evidence source and failed field. `run_integrity` remains `NOT_EVALUATED`.

CLI exit codes are stable: `0` means executable inspection completed with either an exact or unknown build; `1` means a process/probe or executable inspection failure; `2` means invalid CLI usage. Exit `0` does not mean Steam metadata succeeded or that run integrity was evaluated: check `steam_status`, `errors`, and `run_integrity` separately.

The initial registry is empty, so no executable is reported as a known build yet. Build compatibility does not establish run integrity, and neither the local registry nor its report is a tamper-proof attestation. Linux process discovery and game-memory access are not implemented.

The upstream [DESRU repository is marked GPL-3.0](https://github.com/bowsr/DESRU). No DESRU source was copied into this first slice; this checkout does not contain the separate research notes needed to clear their provenance. No project license is selected. Review those notes and any excerpts before publishing under MIT.
