# Mobile UI (Android) — design draft

> Fork experiment (`android-experiment` branch). Not an upstream doc yet: this describes the
> touch-first presentation for phones and tablets, layered on the same engine, commands and
> undo model as the desktop. It will be re-measured against a real device at every phase.

## Principles

1. **One app, two presentations.** The egui app gains a `mobile` presentation mode — same
   engine, same command ids, same undo. Nothing in the editing semantics changes; only the
   chrome does. A tablet can switch back to the desktop presentation (Settings ▸ Appearance).
2. **Touch targets before density.** Every interactive element ≥ 44 dp (`egui::style`
   `interact_size`, larger hit boxes on timeline clips). Precision is a desktop assumption.
3. **Gestures replace precision tools.** One finger = select / scrub; two fingers = pan /
   zoom (timeline and monitors); long-press = context menu (replaces right-click); a clip's
   trim handles get enlarged invisible hit areas (± 12 dp).
4. **Occlusion-aware editing.** A finger hides what it touches: drags move a ghost first and
   commit on lift; the playhead scrubs against a fixed center line (film-wheel style), not a
   thin dragged line.

## Layout

| Device | Presentation |
|---|---|
| Phone (portrait) | Single-pane focus mode: the monitor OR the timeline full-bleed; rails between panes by edge swipe; a bottom tool dock; a top context bar. |
| Tablet (landscape ≥ 10") | The desktop three-pane layout with collapsible side panels and a taller touch toolbar; the exact desktop layout remains available. |

Panels use the existing docking, but mobile mode pins a fixed panel set and hides the rest
behind a bottom sheet.

## Timeline on touch

- Trim / ripple / roll become **explicit tool modes** picked from a tool tray (the CapCut /
  Premiere Rush model), not edge-hover semantics a finger cannot hold.
- **Split** is playhead + one button — no precision required.
- Clip handles keep their desktop semantics (the same commands fire) behind the enlarged hit
  areas and the two-stage ghost drag.

## Monitors

- Full-bleed picture; pinch to zoom, double-tap to fit.
- Transport controls bottom-center, oversized; J/K/L keyboard playback maps to a hold-to-play
  button and a side scrub bar with tick feedback.

## Text entry (the weak link)

egui's IME story on Android is the least mature part of the stack. Rename, captions and
transcription open a **full-screen editor sheet** instead of an inline text field, so all text
input happens in one widget that can be made IME-safe.

## Stylus

Android `MotionEvent` carries pressure and tilt; winit's Android back end forwards pointer
events. The desktop pen pipeline (crates/tablet on Windows/macOS) gains an Android source once
the shell runs — same downstream path.

## Storage

Projects and media move to the Storage Access Framework (document trees); the engine's file
access goes through a small location trait so desktop paths keep working unchanged.

## Phases

1. **Shell** (this branch): `android_main`, the workspace compiling for
   `aarch64-linux-android`, an empty window on a device.
2. **Touch chrome**: interact-size scaling, long-press menus, the bottom dock, mobile mode
   setting.
3. **Monitor-first playback**: single-pane navigation, the scrub bar, transport.
4. **Timeline touch editing**: tool-tray modes, ghost drags, split button.
5. **Storage**: SAF locations for projects and media.
