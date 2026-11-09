Local changes to egui-directx11 0.13.0+egui-0.35.0 (upstream licenses retained):
- Reuse tessellation and immutable vertex/index buffers when clipped shapes,
  target size, pixels-per-point, and zoom match the previous rendered frame.
- Texture deltas still apply every frame; registered video texture content may
  change independently of geometry. Input, animation and resizing invalidate by
  their changed shapes/viewport. No presentation timing changes.
- Expose geometry_cache_stats for performance validation.
- Expose needs_render for unchanged static windows. The application bypasses
  that gate for live video and forces painting for OS expose, focus, startup and
  resize events; managed texture updates also invalidate it.

- Expose per-window GeometryCache swapping so detached windows reuse their own
  immutable geometry while sharing one device, texture pool and decoded frames.

- Import shader-resource views from a renderer on the same device using stable
  user texture aliases. Detached windows now own independent egui input/font
  contexts and renderers while referencing the original decoded video texture;
  no extra video decoding, pixel upload or GPU texture copy is introduced.
