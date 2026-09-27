# #57 eval: more mac-native left-panel rendering

Spike, decided **wontfix** (native interop). Keep the current
`gpui-component` sidebar; fix panel behaviors incrementally in gpui.

## Options considered

1. **Native `NSOutlineView` / `NSTableView` via interop.**
   The pinned `gpui 0.2.2` line exposes no public API for embedding an
   `NSView` as window content (the mac backend owns its content view;
   verified against the vendored `gpui-0.2.2/src/platform/mac/window.rs`).
   Embedding would mean a second overlay `NSWindow` manually tracking the
   panel rect, plus hand-bridged focus between the AppKit responder chain
   and gpui `FocusHandle`s, plus manual theme sync. No public seam —
   only private hacks against a pinned crate.

2. **Better-supported gpui patterns (adopted direction).**
   The panel already uses the component line built for this gpui version
   (`SidebarGroup` / `SidebarMenu` / `SidebarMenuItem` from
   `gpui-component 0.5.x`, the last line on gpui 0.2.2). The same crate
   ships keyboard-navigable `Tree`, `Table`, and virtualized `List`
   (up/down/enter/escape bindings included) if the panel ever outgrows
   the sidebar — migration stays inside the tested gpui event model.

## Against the constraints

- **Keyboard parity:** selection, link-cursor, `/`-filter, `Tab` focus,
  and `?` help are app-owned state shared by keys and mouse. The
  sidebar component carries no key bindings of its own, so parity holds
  by construction today. A native view would own selection/first-
  responder itself and force a parallel keyboard model to be reconciled
  with it — the highest regression risk in this spike.
- **Non-color parity:** row markers, harness badges, and status glyphs
  are text, rendered identically with color removed. Cell-based AppKit
  views would reimplement each marker as view configuration.
- **Offline / local-only:** neutral — both paths are local. No win.
- **Testability:** current panel logic (`is_history`, sections, filter
  matching) is headless unit-tested. AppKit views need a runloop and
  display; the spike would delete coverage, not add it.

## Recommendation

**Wontfix native interop.** Nothing native is clearly better once
keyboard + non-color parity, offline, and testability are priced in.
The "hackish" behaviors named in the issue (hover/selection/scroll/
focus) are mostly the app-owned keyboard-parity state, which native
interop would duplicate rather than remove. Concrete next steps stay
in-stack: keep `Sidebar*` components, reach for component `Tree`/`List`
only if row counts demand virtualization, and fix individual panel
behaviors with headless-tested logic as today.

Revisit if: the framework migrates off gpui 0.2.2 to a line with a
public native-view embedding API, or the panel grows virtualization /
drag-reorder needs the component set cannot meet.
