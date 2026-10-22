# Local eframe 0.33.3 patch

Source: the existing locked crates.io eframe 0.33.3 package. Upstream source commit: 44cdd653e2317d300fb8a6c9c36b03f23991e803, crates/eframe, https://github.com/emilk/egui.

Upstream licensing: MIT OR Apache-2.0. Both license texts are retained from that pinned source commit. Upstream source notices remain intact. SetupScope modifications are identified in the modified source file.

Only src/native/wgpu_integration.rs is changed. Existing root-window AccessKit initialization already uses the event-loop proxy. The patch passes that proxy to child-window initialization, creates each child hidden until its adapter is installed (required by AccessKit), calls init_accesskit on each child egui_winit State, restores the requested visibility, and preserves the existing event dispatch and adapter lifecycle. This covers the WGPU backend used by SetupScope; it does not change Glow or claim browser support.

Reason: live Windows inspection showed child tool windows rendering normally but exposing only the OS title bar in UI Automation. Source inspection confirmed missing init_accesskit for child states. Recheck this patch when updating eframe and remove it once an equivalent upstream fix is integrated. This is a local dependency patch, not an upstream submission or a new framework.

2026-10-22: Windows hidden-parent repaint support in the same source file. Immediate child viewports originally scheduled a redraw on their parent HWND; a hidden parent receives no WM_PAINT, freezing child input and accessibility updates. Hidden-parent wakeups now route through a visible child, and that child's render invokes the hidden parent's update directly. This preserves existing immediate-viewport ownership without keeping a visible blank launcher window. Scope is Windows/WGPU; live hidden-board input, restoration and last-child-close checks are required. Upstream notices remain unchanged.
