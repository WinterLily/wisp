use serde::{Deserialize, Serialize};

pub const FORMAT_VERSION: u32 = 1;
pub const PROGRAM_VERSION: u32 = 4;
pub const SHADER_ABI_VERSION: u32 = 3;
pub const GPU_PROFILE: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format_version: u32,
    pub id: String,
    pub name: String,
    pub author: String,
    pub description: String,
    pub preview: Option<String>,
    pub program: String,
    pub minimum_wisp_version: String,
    pub gpu_profile: u32,
    pub tags: Vec<String>,
}

/// Version 4 adds compute passes and storage buffers to the ordered pass program.
/// Future resource/pass types must be implemented and validated before acceptance.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Program {
    pub version: u32,
    pub shader_abi_version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<crate::Parameter>,
    pub resources: Vec<Resource>,
    pub passes: Vec<Pass>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Resource {
    Image { id: String, path: String },
    RenderTarget { id: String, width: u32, height: u32 },
    StorageBuffer { id: String, elements: u32 },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Pass {
    Compute {
        shader: String,
        #[serde(default)]
        images: Vec<String>,
        #[serde(default)]
        buffers: Vec<StorageBinding>,
        output: String,
        dispatch: [u32; 3],
    },
    Fragment {
        shader: String,
        background: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<String>,
        output: Output,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Output {
    Presentation,
    Target(String),
}

impl Resource {
    pub fn id(&self) -> &str {
        match self {
            Self::Image { id, .. }
            | Self::RenderTarget { id, .. }
            | Self::StorageBuffer { id, .. } => id,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StorageBinding {
    pub resource: String,
    pub writable: bool,
}

impl Pass {
    pub fn shader(&self) -> &str {
        match self {
            Self::Fragment { shader, .. } | Self::Compute { shader, .. } => shader,
        }
    }
    pub fn inputs(&self) -> impl Iterator<Item = &str> {
        let (first, rest) = match self {
            Self::Fragment {
                background, images, ..
            } => (Some(background.as_str()), images),
            Self::Compute { images, .. } => (None, images),
        };
        first.into_iter().chain(rest.iter().map(String::as_str))
    }
}

pub const MAX_STORAGE_BUFFERS: usize = 4;
pub const MAX_STORAGE_ELEMENTS: u32 = 4 * 1024 * 1024;
pub const MAX_DISPATCH_GROUPS: u64 = 1024 * 1024;
pub const MAX_PASSES: usize = 16;
pub const MAX_RENDER_TARGETS: usize = 15;
pub const MAX_TARGET_DIMENSION: u32 = 4096;
