# Wisp's Iced device-limit patch

Source: crates.io `iced_wgpu` 0.14.0, from Iced's 0.14.0 release.
License: MIT (included). Source files are unchanged except:

`src/window/compositor.rs`: request `max_bind_groups: 4` instead of `2`.

Iced 0.14 hardcodes this device limit and exposes no application setting for it.
Wisp's existing portable compute ABI uses groups 0–3. Its embedded preview shares
Iced's device, so the host must request four groups. Both wgpu default and downlevel
default limits already support four. No shader, rendering, or UI code is forked.

Remove this patch when Iced exposes configurable device limits. Keep this source
excluded from the Wisp workspace; it is a patched dependency, not a Wisp crate.
