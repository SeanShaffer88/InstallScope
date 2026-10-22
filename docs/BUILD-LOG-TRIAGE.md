# Bounded build-log triage

Developer diagnostics > Build-log triage (read-only), regular interface.
Select one stable saved local log, then explicitly start triage. Nothing runs on
selection. The source is opened for reading only; no build, script, installer,
engine or suggested command is executed. Nothing is modified or uploaded.

## Bounds and encodings

- Maximum input: 2 MiB. Larger files fail with a request for a smaller excerpt;
  the program does not silently analyze just the first bytes of an oversized file.
- UTF-8 (optional BOM), or BOM-marked UTF-16 little/big endian. Malformed Unicode
  and NUL-containing/binary inputs fail; legacy code pages are not guessed.
- Inspect at most 20,000 lines. Skip individual lines over 8 KiB of decoded UTF-8.
  Partial and skipped-line counts are explicit. Source lines are numbered from 1,
  including skipped lines. Up to five source line numbers are retained per group.
- ANSI CSI/OSC controls are stripped for matching, never executed or linked.
- Fixed 15-category rule table bounds output independently of log size; a match
  counts once per category per line. Multiple categories may match one line.
- Relative, network, device, linked/reparse paths and non-regular files are refused.
  Before/after handle metadata detects observed size/time changes while reading.
  Same-size/same-time rewrites and concurrent path swaps are not a security boundary.
- Cancellation is cooperative per line; an OS file read must return first. A
  cancelled/partial report cannot support a clean/success verdict.

## Interpretation and privacy

Recognized hints include Rust imports/targets/linker discovery; MSVC linker/header
errors; MSBuild toolsets; .NET SDK and NuGet feed failures; TypeScript module lookup;
npm resolution conflicts; Python imports; CMake compiler discovery; disk exhaustion
and access failures. Static guidance includes official reference URLs.

These are offline textual heuristics, not a parser of every tool/version or a root
cause guarantee. Quoted diagnostics can match; translated, multiline, custom,
compressed and unknown formats can be missed. No findings does not prove success.
The utility never downloads a model, accesses a credential/config file, queries a
service, or follows an instruction embedded in a log.

Raw source lines, arbitrary URLs and suggested shell commands are omitted from the
report. The selected source path and observation time are retained. Explicit
clipboard copying is therefore not fully redacted. Review before sharing. Reports
are held in memory, are not automatically saved and are not diagnostic-bundle
sources. Failures retain previous dated evidence instead of presenting it as new.

## Evidence

Four focused tests cover encodings, malformed/binary data, ANSI escapes, private
sentinels, grouping, source line positions, code-token boundaries, all category
patterns, oversized/long/excess lines and cancellation. A live saved-file probe
found three fixture groups while preserving source bytes and modification time;
a private log sentinel was absent from its report. No build was run for the fixture.
The existing JSON/native infrastructure was preserved; no dependency was added.

Live local x64 GUI smoke also passed: select the saved synthetic fixture, run
triage and inspect three grouped categories with correct source line numbers.
A genuine screenshot documents this result; full accessibility/ARM64 acceptance
remains open. Full suite: 442 core, 54 regular UI and 58 native-preview checks
passed; one core and two doc tests ignored.
