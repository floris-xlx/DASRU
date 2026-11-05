# TDA metrics renderer: static data-flow analysis

## Scope and build

This is a read-only static analysis of the installed `DOOMTheDarkAges.exe`. No game process was launched or modified for this analysis.

- Executable: `C:\Program Files (x86)\Steam\steamapps\common\DOOMTheDarkAges\DOOMTheDarkAges.exe`
- PE version: `1.0.0.1`
- SHA-256: `b7aece5fe2842035efd9036a1049edd53ee5efa25564811e20c39ca18362c018`
- Build: Steam `25755602`; this hash matches the local `data/builds.toml` record. Windows Authenticode verification returned `Valid` for id Software LLC.
- Disassembly was taken from this exact file with LLVM `llvm-objdump`.

## Verified static relationships

### Row-object setup and history updates

At module RVA `0x4A1FE0`, setup code uses the incoming `RCX` object (`this`, copied to `RDI`) and stores row-object pointers in slots beginning at `this+0x61E310`. The live discovery artifact maps eight slots at eight-byte intervals to `FPS`, `Total`, `Render`, `Game`, `GPU`, `CPU`, `Session`, and `RS`. This matches the decoded slot loads in routines at RVAs `0x49B280`, `0x49C303`, and `0x49C52E`.

The helper at RVA `0x4A61A0` accesses each row object. Its relevant layout is:

| Row offset | Use in decoded code |
| --- | --- |
| `+0x44` | Disabled flag; zero returns without updating |
| `+0x45` | When set, clamps the incoming float in `XMM2` to `[0, 1]` |
| `+0x88` | History data pointer |
| `+0x90` | History count |
| `+0x94` | History capacity |

The history element stride is `0x48` bytes. Calls at RVAs including `0x49B901`, `0x49BAC8`, `0x49BBC6`, `0x49BC38`, `0x49BCA1`, `0x49C200`, `0x49C636`, `0x49C716`, `0x49C7F3`, and `0x49C901` pass `EDX=-1`; that selects the helper's append/update path. The helper shifts existing elements when needed, writes the current float and associated sample payload, and increments the count. When count reaches capacity, it calls a growth helper at `0x4A5170` before inserting.

This proves the executable contains code that can populate these histories when the row is enabled and the call path runs. It does not prove those calls ran during the saved live capture.

### Four-value formatter

The format string at RVA `0x3351DF8` is:

```text
%s%s%6.2f%s%6.2f%s%6.2f%s%6.2f
```

The call at RVA `0x4A04B1` passes this template to wrapper `0x4A0180`. The call site supplies five string arguments and four floating-point values; the four floats are converted to doubles and interleaved with the string arguments. The first string is also supplied as the final string argument. The caller compares input floats against thresholds and selects string values before formatting. The semantic names of those four values and their connection to the row-history arrays are not established by this trace.

Wrapper `0x4A0180` reserves `0x870` bytes of stack, passes a buffer at `RSP+0x40` with a maximum length of `0x800` to bounded formatter `0x2148F80`, then passes that temporary buffer to text helper `0x90FDE0` and text/glyph helper `0x90FB00`.

### FPS and frametime formatting

The adjacent templates begin at RVAs `0x3351E18` and `0x3351E20`:

```text
0x3351E18  %i FPS\0\0
0x3351E20  %.2fms\0\0
```

In the code around RVA `0x49C212`, the FPS formatting path is gated by `R12D >= 1`. It loads a pointer from the global at RVA `0x91BCEF0`, reads the integer at `[pointer+0x60]`, places it in the variadic argument area, selects the `%i FPS` template, and calls wrapper `0x4A0290` at `0x49C24A`.

The frametime path is gated by `R12D >= 2`. A call to `0x20B2100` supplies a float return value, which is converted to double and passed with the `%.2fms` template to wrapper `0x4A0290` at `0x49C2A0`.

Wrapper `0x4A0290` also reserves `0x870` stack bytes and formats into a `0x800`-byte buffer at `RSP+0x40`. It passes that buffer to the same text-measure and text/glyph helpers. The disassembly identifies a glyph/batch path, but this analysis did not identify the final graphics submission.

## Runtime observations from the saved capture

[`metrics-gameplay-history-final.json`](../metrics-gameplay-history-final.json) records an exact-build process inspection with full committed-readable image/private/mapped scan coverage, no failed reads, eight row histories, and five samples at 250 ms intervals. Each history had count and capacity `180`; no pointer, count, capacity, or sampled element changes were reported. The report records `memory_modified: false`.

Two limits matter when interpreting that result:

1. `metrics_display_state` is `NOT_OBSERVED` in the JSON, so the artifact itself does not establish that the metrics display was enabled throughout sampling. The user reported active gameplay for the capture, but that state was not machine-verified.
2. Header and array reads are separate operations, so each sample is not an atomic snapshot. The capture also records `runtime_invocation: NOT_OBSERVED` for the update helper.

Thus the unchanged sample is evidence only that no changes were observed in those reads. It does not show that the histories are unused or that the update code did not execute between samples.

## Conclusion

The static evidence confirms row setup, guarded history updates, four-value formatting, and separate FPS/frametime formatting into temporary stack buffers that feed text/glyph helpers. It does **not** establish a path from the `+0x88` history arrays into the formatted text, nor does it prove that the target routine is invoked during visible OSD rendering. Keep `renderer_consumption` as `RUNTIME_RENDERER_CONSUMPTION_UNCONFIRMED` and structural confidence at `MEDIUM_STRUCTURAL`.

No further generic history sampling is likely to resolve that gap. The decisive next evidence would be either runtime observation of the formatter/update call path or a static data-dependency trace from history elements into the formatter arguments and then into the glyph batch.

## Reproduction notes

Static addresses are RVAs and therefore specific to the verified executable hash above. The live capture is process-specific and was not treated as an atomic or complete execution trace. No tests were run; this change adds an analysis report only.
