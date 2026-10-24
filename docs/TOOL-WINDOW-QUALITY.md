# Tool Workflow Audit

The board contains five independent, compact windows. Readiness owns requirements, toolchain evidence, redacted export and the separately reviewed dependency manager. Each window has a dedicated instrument mark, workflow cues and permission boundary. Tool bodies remain tailored to their task.

2026-10-22: native workload/layout options remain collapsed inside Dependencies; build/test execution is a separate collapsed reviewed workflow inside Troubleshooting. Inputs expose names/tooltips, local paths/arguments are validated, and edits clear approval. A live multi-window check found shared header state clipping readiness tabs: headers now have tool-specific panel IDs, with a painted-navigation clipping regression test. Live checks verify the tabs remain visible with Troubleshooting open. Some UI Automation pointer geometry was stale across child windows; screenshot-based coordinates were used where needed. This is not actual screen-reader certification.

| Tool | Controls and evidence | Changing boundary / recovery |
| --- | --- | --- |
| Project Readiness | Explicit project, target/profile, optional saved log; bounded observations and evidence handoff | Dependencies requires download actions, pinned verification, exact install review and receipts. Export requires exact-preview review. |
| Build Troubleshooting | Reviewed executable/arguments, timeout and captured logs; bounded saved-log hints | Explicit acknowledgment before project code executes. Cancellation stops contained children, not side effects. Export refuses overwrite. |
| Environment Profiles | Project paths/variables, preview, child launch, persistent edit | Launch and persistent changes have separate reviews. Undo refuses conflicts. |
| Ports And Processes | Explicit listeners/file owners, filters and paging | No termination, unlocking, connections or firewall changes. |
| WSL And Containers | Explicit passive host/CLI/service evidence | No engine, distribution, container or service starts. |

Heights now follow measured scroll contents on opening and section/result changes, capped to the monitor. Normal frame padding is retained; deliberate manual resizing persists until contents change. Initial hints (logical pixels) are board 960×600, Readiness 760×360, Troubleshooting 720×250, Environment 760×500, Ports 780×460, Containers 660×250, Settings 580×250. These are initial hints, not fixed heights. Each window has a smaller minimum; controls wrap and long reports/reviews scroll.

Automated checks cover five-tool ownership/singletons, repeated opens, minimum-width controls, geometry on smaller displays, dependency selection and consent reset. Live checks include the compact windows, synthetic logs/projects, process-local environment preview, listener/file-owner observations, passive host inspection, export review and dependency verification/review preparation. Persistent user-environment edits and real vendor installation are tested through fixtures/mocks rather than changes to unrelated user state.

See RELEASE-VALIDATION.md for current build evidence and exact limits; this audit is not a claim of compatibility with untested systems or every assistive technology.

2026-10-22: board changed to a 3×2 grid (five tools plus passive system card); tool return buttons aligned top-right. Automated layout tests cover the grid at 740/960 logical pixels and all tool return buttons at minimum widths. Refreshed live screenshots of the board, every tool window and the language previews are stored in `docs/screenshots` and referenced by the README.

2026-10-22 localization polish: owned section headings pre-layout wrapped galleys because the stock collapsing header forces nonwrapping text. Compact preferred/minimum-width checks now cover every language in isolated processes with configuration sections expanded. Diagnostic previews use bounded content-driven heights, avoiding shrink feedback from the previous viewport. The environment PATH editor uses available width. Actual screen-reader/clean-machine certification remains outside this evidence.
