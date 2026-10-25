use bytemuck::{Pod, Zeroable};

/// Shader ABI 1 globals. Coordinates are normalized, with the origin at top left.
/// Hosts supply normalized audio bands; capture remains outside the runtime.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Globals {
    pub resolution: [f32; 2],
    pub mouse: [f32; 2],
    pub time: f32,
    pub delta_time: f32,
    pub audio_bass: f32,
    pub audio_mid: f32,
    pub audio_treble: f32,
    pub audio_volume: f32,
    pub frame: u32,
    pub(crate) padding: u32,
}

impl Globals {
    /// Band order: bass, mid, treble, volume. Invalid host values become silence.
    pub fn set_audio(&mut self, bands: [f32; 4]) {
        let [bass, mid, treble, volume] =
            bands.map(|v| if v.is_finite() { v.clamp(0., 1.) } else { 0. });
        self.audio_bass = bass;
        self.audio_mid = mid;
        self.audio_treble = treble;
        self.audio_volume = volume;
    }
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            resolution: [width as f32, height as f32],
            mouse: [0.5, 0.5],
            ..Self::zeroed()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_layout_matches_wgsl_abi() {
        assert_eq!(size_of::<Globals>(), 48);
        assert_eq!(std::mem::offset_of!(Globals, time), 16);
        assert_eq!(std::mem::offset_of!(Globals, audio_treble), 32);
        assert_eq!(std::mem::offset_of!(Globals, frame), 40);
        let shader = crate::Shader::new(crate::EXAMPLE_SHADER).unwrap();
        let module = naga::front::wgsl::parse_str(shader.source()).unwrap();
        let (_, ty) = module
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref() == Some("Globals"))
            .unwrap();
        let naga::TypeInner::Struct { members, span } = &ty.inner else {
            panic!("Globals must be a struct")
        };
        assert_eq!(*span, size_of::<Globals>() as u32);
        assert_eq!(
            members.iter().map(|m| m.offset).collect::<Vec<_>>(),
            [0, 8, 16, 20, 24, 28, 32, 36, 40]
        );
    }
}
