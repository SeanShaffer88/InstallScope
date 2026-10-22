# Desktop child-window accessibility patch

SetupScope uses the existing eframe 0.33.3 dependency through a local, source-retaining patch in vendor/eframe-0.33.3. Its WGPU child windows now initialize the same AccessKit integration used by the root window. The prior backend exposed only native window chrome to UI Automation in live checks.

The same WGPU integration file also routes hidden-parent redraws through visible child windows on Windows. Immediate viewports need the parent's update; requesting WM_PAINT on a hidden parent otherwise freezes tool input and accessibility updates. This supports hiding the Workflow Board while independent tools remain active.

See vendor/eframe-0.33.3/SETUPSCOPE-PATCH.md for provenance, precise scope and upgrade/removal guidance. Upstream MIT and Apache-2.0 notices are retained and must accompany local packaging. The rest of the UI framework family remains unchanged. Screen-reader/DPI acceptance must still be reported separately from adapter initialization or automated accessibility-tree checks.
