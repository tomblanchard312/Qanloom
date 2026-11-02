//! Curated, verified image bundles: Z-Image-Turbo for CPU and Qwen-Image-2512 for GPU.
//! FLUX constants remain for existing clients and explicit model choices.

/// Registry id of the diffusion file, which is its name without `.gguf`.
pub const DEFAULT_IMAGE_MODEL_ID: &str = "flux-2-klein-4b-Q4_0";
pub const DEFAULT_IMAGE_LICENSE: &str = "Apache-2.0";
/// Distilled klein sampling: 4 steps, guidance 1, trained size 1024.
pub const DEFAULT_IMAGE_STEPS: u32 = 4;
pub const DEFAULT_IMAGE_CFG: f32 = 1.0;
pub const DEFAULT_IMAGE_SIDE: u32 = 1024;

/// Which piece of the bundle a file is. Only the diffusion file is registered as a model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DefaultImageRole {
    Diffusion,
    Vae,
    TextEncoder,
}

pub struct DefaultImageFile {
    pub role: DefaultImageRole,
    /// Path relative to the `diffusion` directory in a models folder.
    pub relative: &'static str,
    pub url: &'static str,
    /// Lowercase SHA-256 of the file bytes.
    pub sha256: &'static str,
    pub size: u64,
}

pub const DEFAULT_IMAGE_FILES: [DefaultImageFile; 3] = [
    DefaultImageFile {
        role: DefaultImageRole::Diffusion,
        relative: "flux-2-klein-4b-Q4_0.gguf",
        url: "https://huggingface.co/leejet/FLUX.2-klein-4B-GGUF/resolve/main/flux-2-klein-4b-Q4_0.gguf",
        sha256: "d1023499ef3f2f82ff7c50e6778495195c1b6cc34835741778868428111f9ff4",
        size: 2_460_378_560,
    },
    DefaultImageFile {
        role: DefaultImageRole::Vae,
        relative: "flux-2-klein-4b/vae.safetensors",
        url: "https://huggingface.co/black-forest-labs/FLUX.2-klein-4B/resolve/main/vae/diffusion_pytorch_model.safetensors",
        sha256: "ca70d2202afe6415bdbcb8793ba8cd99fd159cfe6192381504d6c4d3036e0f04",
        size: 168_120_878,
    },
    DefaultImageFile {
        role: DefaultImageRole::TextEncoder,
        relative: "flux-2-klein-4b/llm.gguf",
        url: "https://huggingface.co/unsloth/Qwen3-4B-GGUF/resolve/main/Qwen3-4B-Q4_K_M.gguf",
        sha256: "f6f851777709861056efcdad3af01da38b31223a3ba26e61a4f8bf3a2195813a",
        size: 2_497_281_312,
    },
];

/// Sidecar next to the diffusion file. The image worker reads it to find the VAE and text encoder.
pub const DEFAULT_IMAGE_BUNDLE: &str = "flux-2-klein-4b-Q4_0.gguf.bundle.json";

/// The user's models folder: `$XDG_DATA_HOME/SharedAI/models`, else
/// `~/.local/share/SharedAI/models`. On Windows, `%LOCALAPPDATA%\SharedAI\models`.
pub fn user_models_dir() -> Option<std::path::PathBuf> {
    #[cfg(unix)]
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|d| !d.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")));
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from);
    base.filter(|b| b.is_absolute()).map(|b| b.join("SharedAI").join("models"))
}

/// Curated image recommendations. Updating Enki updates this catalog; older bundles stay on disk.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ImageProfile {
    pub model: String,
    pub side: u32,
    pub steps: u32,
    pub cfg: f32,
}

pub const CPU_IMAGE_MODEL_ID: &str = "z-image-turbo-Q4_0";
pub const GPU_IMAGE_MODEL_ID: &str = "qwen-image-2512-Q4_K_M";

pub fn image_profile(model: &str) -> Option<ImageProfile> {
    let (side, steps, cfg) = match model {
        CPU_IMAGE_MODEL_ID => (512, 8, 1.0),
        GPU_IMAGE_MODEL_ID => (1024, 50, 2.5),
        DEFAULT_IMAGE_MODEL_ID => (DEFAULT_IMAGE_SIDE, DEFAULT_IMAGE_STEPS, DEFAULT_IMAGE_CFG),
        _ => return None,
    };
    Some(ImageProfile { model: model.into(), side, steps, cfg })
}

/// Image jobs can have long CPU steps. Text retains its shorter deadlines.
pub const IMAGE_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60);
pub const IMAGE_WALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(4 * 60 * 60);
pub const CPU_IMAGE_FILES: [DefaultImageFile; 3] = [
    DefaultImageFile {
        role: DefaultImageRole::Diffusion,
        relative: "z-image-turbo-Q4_0.gguf",
        url: "https://huggingface.co/leejet/Z-Image-Turbo-GGUF/resolve/c61c0e422dc8b541b7548cf33a4ef8302b0f8085/z_image_turbo-Q4_0.gguf",
        sha256: "2bc57986874c84f7ec6d02d9d7070a53b0029954a0e38a6e1342eb91095572f5",
        size: 3683370944,
    },
    DefaultImageFile {
        role: DefaultImageRole::TextEncoder,
        relative: "z-image-turbo/llm.gguf",
        url: "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/a06e946bb6b655725eafa393f4a9745d460374c9/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        sha256: "3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597",
        size: 2497281120,
    },
    DefaultImageFile {
        role: DefaultImageRole::Vae,
        relative: "z-image-turbo/vae.safetensors",
        url: "https://huggingface.co/Comfy-Org/z_image_turbo/resolve/6fc90a3b1b653e935a0d175e260736de25b84df5/split_files/vae/ae.safetensors",
        sha256: "afc8e28272cd15db3919bacdb6918ce9c1ed22e96cb12c4d5ed0fba823529e38",
        size: 335304388,
    },
];
pub const GPU_IMAGE_FILES: [DefaultImageFile; 3] = [
    DefaultImageFile {
        role: DefaultImageRole::Diffusion,
        relative: "qwen-image-2512-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen-Image-2512-GGUF/resolve/1626d7531f84b4d2ea1cd6d2e69f41ec027dd354/qwen-image-2512-Q4_K_M.gguf",
        sha256: "b2a5f6249eb58ee10c9e2ce8cb1114b89897db23de2fdf7dc49140800aa928fc",
        size: 13244758560,
    },
    DefaultImageFile {
        role: DefaultImageRole::TextEncoder,
        relative: "qwen-image-2512/llm.gguf",
        url: "https://huggingface.co/mradermacher/Qwen2.5-VL-7B-Instruct-GGUF/resolve/cfa2baa09946b211c107e6e104948987a64dd2c1/Qwen2.5-VL-7B-Instruct.Q4_K_M.gguf",
        sha256: "0f00a930ba3108b6861ddadf74d8ebbd82e257c63eba728e62c3e8970f5eed94",
        size: 4683072512,
    },
    DefaultImageFile {
        role: DefaultImageRole::Vae,
        relative: "qwen-image-2512/vae.safetensors",
        url: "https://huggingface.co/Comfy-Org/Qwen-Image/resolve/1f12b17be14c89b026c51a91d67c32f84bb047bc/split_files/vae/qwen_image_vae.safetensors",
        sha256: "a70580f0213e67967ee9c95f05bb400e8fb08307e017a924bf3441223e023d1f",
        size: 253806246,
    },
];

pub fn image_files(model: &str) -> Option<&'static [DefaultImageFile]> {
    match model {
        CPU_IMAGE_MODEL_ID => Some(&CPU_IMAGE_FILES),
        GPU_IMAGE_MODEL_ID => Some(&GPU_IMAGE_FILES),
        DEFAULT_IMAGE_MODEL_ID => Some(&DEFAULT_IMAGE_FILES),
        _ => None,
    }
}

#[cfg(test)]
mod profiles_tests {
    use super::*;
    #[test]
    fn recommendations_have_verified_complete_bundles() {
        for id in [CPU_IMAGE_MODEL_ID, GPU_IMAGE_MODEL_ID] {
            let p = image_profile(id).unwrap();
            assert!(p.steps > 0 && p.cfg >= 1.0);
            let files = image_files(id).unwrap();
            assert_eq!(files.len(), 3);
            for file in files {
                assert_eq!(file.sha256.len(), 64);
                assert!(!file.url.contains("/main/"));
                assert!(file.size > 0);
            }
        }
        assert_eq!(image_profile(CPU_IMAGE_MODEL_ID).unwrap().steps, 8);
        assert_eq!(image_profile(GPU_IMAGE_MODEL_ID).unwrap().cfg, 2.5);
        assert!(image_profile("unknown").is_none());
    }
}

/// A catalog supplied by the installed daemon, so recommendations can change without app releases.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ImageCatalog {
    pub profile: ImageProfile,
    pub license: String,
    pub files: Vec<ImageFile>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ImageFile {
    pub role: DefaultImageRole,
    pub relative: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

pub fn image_catalog(model: &str) -> Option<ImageCatalog> {
    Some(ImageCatalog {
        profile: image_profile(model)?,
        license: DEFAULT_IMAGE_LICENSE.into(),
        files: image_files(model)?
            .iter()
            .map(|file| ImageFile {
                role: file.role,
                relative: file.relative.into(),
                url: file.url.into(),
                sha256: file.sha256.into(),
                size: file.size,
            })
            .collect(),
    })
}
