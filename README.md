# DASRU

DOOM: The Dark Ages SpeedRun Utility. The first slice is a read-only Windows x64 identity probe.

```text
dasru probe
dasru probe --json
dasru inspect --exe "C:\path\to\DOOMTheDarkAges.exe"
dasru inspect --exe "C:\path\to\DOOMTheDarkAges.exe" --json
dasru memory scan --metrics
dasru memory scan --metrics --json
dasru memory inspect --metrics
dasru memory inspect --metrics --json
dasru memory inspect --metrics --fps-label "%i FPS+" --json
```

`inspect --exe` hashes and reads version metadata from an executable without looking for a running process. Its report uses `PROCESS_NOT_EVALUATED` semantics (`process_status: NOT_EVALUATED`), with an empty process list. `probe` discovers a process by executable name, retains its Windows process handle while it hashes the executable, and reports the live process identity.

JSON reports use `schema_version: 1`; `observed_at_unix_ms` is the UTC Unix epoch time in milliseconds. `status` reports the overall executable/process result. `executable_status` and `steam_status` keep executable recognition separate from Steam metadata. The Steam manifest is checked for App ID, `installdir`, and Build ID format; its resolved install directory must contain the inspected executable. A registered Steam Build ID is compared separately, so a mismatch does not erase a known executable hash. Field provenance and structured errors identify the evidence source and failed field. `run_integrity` remains `NOT_EVALUATED`.

CLI exit codes are stable: `0` means executable inspection completed with either an exact or unknown build; `1` means a process/probe or executable inspection failure; `2` means invalid CLI usage. Exit `0` does not mean Steam metadata succeeded or that run integrity was evaluated: check `steam_status`, `errors`, and `run_integrity` separately.

The initial registry is empty, so no executable is reported as a known build yet. Build compatibility does not establish run integrity, and neither the local registry nor its report is a tamper-proof attestation. Linux process discovery and game-memory access are not implemented.

`memory scan --metrics` scans the main module of the single verified process for the shared FPS format signature. It requests `PROCESS_QUERY_INFORMATION` and `PROCESS_VM_READ` only, walks committed readable regions with `VirtualQueryEx`, and reads them with `ReadProcessMemory`. Guarded, inaccessible, and uncommitted regions are skipped and counted. The report includes every match with its absolute and module-relative address, memory region type, and protection. `bytes_read` counts bytes returned by Windows, `bytes_scanned` counts bytes accepted as complete evidence, `bytes_untrusted` counts returned bytes from failed reads that were excluded, and `bytes_unreadable` counts missing or skipped bytes. `coverage` is `FULL_MODULE` only when every module byte was scanned successfully; otherwise the status is `PARTIAL_SCAN`, with any observed matches retained. A unique match is reported only after complete coverage; ambiguous matches and process/scan errors return exit code `1`. Failed or short reads do not contribute bytes to signature matches.

`memory inspect --metrics` performs the same complete, read-only scan, then reads a bounded context around one unique match. It reports the exact context bytes, region type and protection, the starts and terminators of the three format strings, and conservative byte capacities. Its dry-run defaults to `%i FPS*`; `--fps-label` accepts an ASCII candidate that preserves the `%i FPS` prefix, such as `%i FPS+`. It lists changed bytes only when the complete replacement and terminator fit inside the first string's slot. `SAFE` means only that this in-memory byte diff stays within the observed slot; it does not authorize or perform a write. `UNSUPPORTED` means the layout or replacement cannot be established safely, and `AMBIGUOUS` means multiple process or signature candidates were found. A completed unique scan exits `0` even when the replacement is unsupported; incomplete scans and process errors exit `1`. This command never writes to game memory; run integrity remains `NOT_EVALUATED`.

The upstream [DESRU repository is marked GPL-3.0](https://github.com/bowsr/DESRU). No DESRU source was copied into this first slice; this checkout does not contain the separate research notes needed to clear their provenance. No project license is selected. Review those notes and any excerpts before publishing under MIT.
