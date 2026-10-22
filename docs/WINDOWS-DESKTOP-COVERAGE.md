# Windows desktop development coverage map

The product target is practical coverage of the Windows software development lifecycle. This is a gap map, not a statement that every capability is implemented. Keep the five focused workflows; add built-in evidence, explicit specialist integrations or contextual guidance where each fits. Do not silently install tools, run project code, use certificates or transmit project data.

| Developer need | Current coverage | Next integration within existing workflows |
| --- | --- | --- |
| Native C++/MSVC, CMake, Rust | Bounded prerequisite, declaration, PATH and metadata evidence | Improve compiler/workload selection and target-specific evidence |
| Windows SDK and x64/ARM64 | Target selection and bounded SDK layout observations | Cross-target compiler, libraries and runtime/deployment distinctions |
| .NET desktop, WinForms, WPF, WinUI | Bounded .NET declarations and toolchain evidence | Desktop workload/runtime and framework-specific readiness |
| Other desktop stacks and embedded web UIs | Some generic command/project metadata evidence | Explicit stack presets and runtime/deployment guidance |
| Project setup and reproducibility | Readiness workflow and environment profiles | Shared project context, saved presets, prioritized blockers |
| Source control and collaboration | Git command candidate evidence only | Repository health/readiness guidance and specialist shortcuts |
| Dependencies, native DLLs and ABI | Bounded native requirement observations | Binary architecture, missing dependency and runtime evidence |
| Build failures and logs | Bounded saved-log patterns with source line references | Broader desktop-specific patterns and clearer next actions |
| Debuggers, symbols and crash analysis | No integrated debugger or dump analysis | Debugging readiness and explicit handoff to specialist tools |
| Performance, memory and tracing | No integrated profiler/trace analysis | Profiling readiness and scoped external integrations |
| Unit, integration and UI tests | No project test execution | Discover declared test tooling; reviewed optional execution requires separate design |
| Accessibility, DPI and localization | App-level accessibility work; no project audit | Desktop quality checklist and specialist validation guidance |
| Graphics, media and device integration | No project-specific audit | SDK/runtime/dependency guidance for selected project needs |
| COM, WinRT, IPC and services | No general IPC/service debugger | Platform integration readiness and explicit specialist handoffs |
| Networking, local endpoints and file holders | Passive listener and selected-file-owner observations | Improve evidence filtering and linking to project failures |
| WSL and container development | Passive host readiness only | Preserve runtime-health uncertainty; project-specific requirement guidance |
| Environment configuration and recovery | Process-local preview, reviewed launch/persistent edits, conflict-checked undo | Reuse explicit project context and compare before/after evidence |
| Packaging, deployment and updates | No current installer/package authoring | Packaging prerequisites and deployment-model readiness |
| Signing and release integrity | No certificate/signing automation | Signing-tool/readiness guidance; never handle signing keys implicitly |
| Security, secrets and supply-chain checks | Allowlisted diagnostic export; no comprehensive project security scanner | Reviewed specialist integrations and clear coverage/permission limits |
| Licenses and redistribution | Project dependency provenance maintained | Project-specific license/runtime redistribution guidance |
| CI and repeatable builds | No project CI configuration authoring | Readiness report and explicit reproducibility/export guidance |
| Diagnostics and support reports | Bounded summaries, copies and redacted JSON sections | Selected findings across all five tools and before/after comparison |

## Delivery order

1. Shared explicit project context and prioritized findings.
2. Broader Windows SDK/compiler/.NET desktop readiness and native dependency evidence.
3. Debugging, test, packaging and signing readiness with contextual specialist handoffs.
4. Before/after comparison and unified selected-findings export.
5. Supported-platform, accessibility and release validation.

Every increment needs a stated scope, configurable inputs where useful, read-only defaults, explicit reviews for changes and practical recovery. External resources remain candidates; consult the saved resource catalog and verify upstream support/licensing before adoption. No new standalone tool category is needed merely to expose a relevant guide or integration.
