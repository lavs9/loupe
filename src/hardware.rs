use serde::{Deserialize, Serialize};

/// System hardware tier categorizing available Apple Silicon Unified Memory or RAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HardwareTier {
    /// Tier 1: Base (≤ 16 GB Unified Memory)
    /// Typical devices: MacBook Air 8/16GB, MacBook Pro 14" Base M1/M2/M3/M4.
    Base16Gb,
    /// Tier 2: Performance (20 GB – 36 GB Unified Memory)
    /// Typical devices: M4 Pro 24GB, M2/M3 Pro 18/36GB, M1/M2 Max 32GB.
    Performance24To32Gb,
    /// Tier 3: Workstation (≥ 48 GB Unified Memory)
    /// Typical devices: M1/M2/M3/M4 Max 48GB/64GB/128GB, M1/M2 Ultra.
    Workstation48GbPlus,
}

impl HardwareTier {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Base16Gb => "Tier 1: Base (≤ 16 GB Unified Memory)",
            Self::Performance24To32Gb => "Tier 2: Performance (20 – 36 GB Unified Memory)",
            Self::Workstation48GbPlus => "Tier 3: Workstation (≥ 48 GB Unified Memory)",
        }
    }

    pub fn short_name(&self) -> &'static str {
        match self {
            Self::Base16Gb => "Tier 1: Base",
            Self::Performance24To32Gb => "Tier 2: Performance",
            Self::Workstation48GbPlus => "Tier 3: Workstation",
        }
    }

    pub fn badge_label(&self) -> &'static str {
        match self {
            Self::Base16Gb => "Tier 1 (Base)",
            Self::Performance24To32Gb => "Tier 2 (Performance)",
            Self::Workstation48GbPlus => "Tier 3 (Workstation)",
        }
    }
}

/// Lifecycle status for an embedding model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelStatus {
    /// Currently shipping active default for all disks
    ActiveDefault,
    /// Supported alternative (e.g. un-truncated 768-d Matryoshka)
    SupportedAlternative,
    /// Planned roadmap model (e.g. GitHub #5 Qwen3-VL 2B)
    RoadmapVision,
    /// Planned roadmap omnimodal model (e.g. GitHub #6 jina-embeddings-v5-omni)
    RoadmapOmni,
}

impl ModelStatus {
    pub fn badge_label(&self) -> &'static str {
        match self {
            Self::ActiveDefault => "Active Default",
            Self::SupportedAlternative => "Supported Alternative",
            Self::RoadmapVision => "Roadmap (Issue #5)",
            Self::RoadmapOmni => "Roadmap (Issue #6)",
        }
    }
}

/// Model specification describing an embedding model's dimensions, footprint, and suitability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub size_label: &'static str,
    pub dimensions: usize,
    pub memory_footprint: &'static str,
    pub status: ModelStatus,
    pub portability: &'static str,
    pub description: &'static str,
}

/// Detected system hardware specs and intelligent model recommendation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareProfile {
    pub total_memory_bytes: u64,
    pub total_memory_gb: f64,
    pub cpu_brand: String,
    pub model_identifier: String,
    pub cpu_cores: u32,
    pub is_apple_silicon: bool,
    pub tier: HardwareTier,
}

impl HardwareProfile {
    /// Detect system hardware on the current Mac or fallback on other OSes.
    pub fn detect() -> Self {
        #[cfg(target_os = "macos")]
        {
            let total_memory_bytes = sysctl_u64("hw.memsize").unwrap_or(16 * 1024 * 1024 * 1024);
            let total_memory_gb = (total_memory_bytes as f64) / (1024.0 * 1024.0 * 1024.0);
            let cpu_brand = sysctl_string("machdep.cpu.brand_string")
                .unwrap_or_else(|| "Apple Silicon".to_string());
            let model_identifier = sysctl_string("hw.model")
                .unwrap_or_else(|| "Mac".to_string());
            let cpu_cores = sysctl_u32("hw.ncpu").unwrap_or(8);
            let is_apple_silicon = cpu_brand.to_ascii_lowercase().contains("apple")
                || std::env::consts::ARCH == "aarch64";

            let tier = Self::classify_tier(total_memory_gb);

            Self {
                total_memory_bytes,
                total_memory_gb,
                cpu_brand,
                model_identifier,
                cpu_cores,
                is_apple_silicon,
                tier,
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let total_memory_bytes = 16 * 1024 * 1024 * 1024;
            let total_memory_gb = 16.0;
            let cpu_brand = "Generic CPU".to_string();
            let model_identifier = "Generic Workstation".to_string();
            let cpu_cores = 8;
            let is_apple_silicon = false;
            let tier = HardwareTier::Base16Gb;

            Self {
                total_memory_bytes,
                total_memory_gb,
                cpu_brand,
                model_identifier,
                cpu_cores,
                is_apple_silicon,
                tier,
            }
        }
    }

    /// Classify hardware tier based on gigabytes of unified memory.
    pub fn classify_tier(gb: f64) -> HardwareTier {
        if gb <= 18.0 {
            HardwareTier::Base16Gb
        } else if gb < 40.0 {
            HardwareTier::Performance24To32Gb
        } else {
            HardwareTier::Workstation48GbPlus
        }
    }

    /// Primary recommended model ID.
    ///
    /// Portability Rule (MODELS.md):
    /// Catalogs are tied to the model ID in `.photo_index/config.toml`.
    /// The universal portable default is always `EmbeddingGemma 2 740M LiteRT`,
    /// ensuring that external drives indexed on any Mac open and search on any other Mac.
    pub fn recommended_model_id(&self) -> &'static str {
        crate::embedder::MODEL_ID
    }

    /// Name of the recommended model.
    pub fn recommended_model_name(&self) -> &'static str {
        "EmbeddingGemma 2 740M LiteRT (256-d Matryoshka)"
    }

    /// Explanation of why this model is recommended for the detected hardware.
    pub fn recommendation_reason(&self) -> &'static str {
        match self.tier {
            HardwareTier::Base16Gb => {
                "Fits comfortably within available unified memory (~0.5–2.0 GB peak). Leaves ample headroom for macOS and apps without memory pressure during background indexing."
            }
            HardwareTier::Performance24To32Gb => {
                "Substantial unified memory headroom for 100k+ photo libraries with zero memory contention. Preserves universal drive portability so external SSDs work seamlessly across all Macs."
            }
            HardwareTier::Workstation48GbPlus => {
                "Workstation-tier memory bandwidth. Universal portable default ensures cross-Mac drive compatibility, while providing massive headroom for concurrent high-throughput indexing."
            }
        }
    }

    /// Capability and headroom description for high-performance options.
    pub fn high_perf_options_reason(&self) -> &'static str {
        match self.tier {
            HardwareTier::Base16Gb => {
                "Base memory tier. Background indexing runs lean; larger vision models would exceed recommended memory boundaries."
            }
            HardwareTier::Performance24To32Gb => {
                "Your hardware can optionally support full 768-d Matryoshka embeddings or opt-in 2B vision models per catalog."
            }
            HardwareTier::Workstation48GbPlus => {
                "Workstation tier can easily handle high-throughput parallel batch indexing, 768-d un-truncated embeddings, or omnimodal models."
            }
        }
    }

    /// Catalog of current and candidate models matching MODELS.md.
    pub fn model_catalog(&self) -> Vec<ModelSpec> {
        vec![
            ModelSpec {
                id: crate::embedder::MODEL_ID,
                name: "EmbeddingGemma 2 740M LiteRT",
                size_label: "~485 MB (.litertlm) + 138 MB runtime",
                dimensions: 256,
                memory_footprint: "~0.5 – 2.0 GB Unified Memory",
                status: ModelStatus::ActiveDefault,
                portability: "Universal Portable Default (Cross-Mac)",
                description: "Multimodal photo, video, audio, and text embedding via Metal. Default model that preserves drive portability.",
            },
            ModelSpec {
                id: "embeddinggemma-2-litert-740m-768",
                name: "EmbeddingGemma 2 740M (Full 768-d)",
                size_label: "~485 MB (.litertlm) + 138 MB runtime",
                dimensions: 768,
                memory_footprint: "~0.8 – 2.5 GB Unified Memory",
                status: ModelStatus::SupportedAlternative,
                portability: "Catalog Specific (Per folder)",
                description: "Full 768-dimension vectors without Matryoshka truncation. Less truncation for massive photo collections.",
            },
            ModelSpec {
                id: "qwen3-vl-embedding-2b",
                name: "Qwen3-VL-Embedding 2B",
                size_label: "~2.2 GB (Apache 2.0)",
                dimensions: 1536,
                memory_footprint: "~3.5 – 6.0 GB Unified Memory",
                status: ModelStatus::RoadmapVision,
                portability: "Catalog Specific (Opt-in per folder)",
                description: "Stronger visual document & photo comprehension scores (MMEB). Recommended for 32 GB+ Macs (Issue #5).",
            },
            ModelSpec {
                id: "jina-embeddings-v5-omni-1.7b",
                name: "jina-embeddings-v5-omni 1.7B",
                size_label: "~2.5 GB omnimodal weights",
                dimensions: 1024,
                memory_footprint: "~4.0 – 8.0 GB Unified Memory",
                status: ModelStatus::RoadmapOmni,
                portability: "Catalog Specific (Opt-in per folder)",
                description: "Unified text, image, video, audio, and PDF embeddings for large-scale workstation libraries (Issue #6).",
            },
        ]
    }

    /// Formatted report for CLI `loupe hardware` / `loupe sysinfo`.
    pub fn cli_report(&self) -> String {
        let memory_label = format!("{:.1} GB ({} bytes)", self.total_memory_gb, self.total_memory_bytes);
        let chip_label = format!("{} ({} cores)", self.cpu_brand, self.cpu_cores);
        let arch_label = if self.is_apple_silicon {
            "Apple Silicon (Unified Memory Architecture)"
        } else {
            "x86_64 / Discrete Memory"
        };

        format!(
r#"Mac Hardware & Memory Profile
────────────────────────────────────────────────────────────────────────────
Chip:                {chip_label}
Model Identifier:    {model}
Architecture:        {arch_label}
Unified Memory:      {memory_label}
Hardware Tier:       {tier}

Intelligent Model Recommendation
────────────────────────────────────────────────────────────────────────────
Recommended Model:   {rec_model}
Model Identifier:    {rec_id}
Recommendation:      [Optimal Portable Default]
Reason:              {reason}

High-Capacity Headroom:
                     {headroom}

Universal Drive Portability Rule:
  External drives indexed with EmbeddingGemma 2 open and search on any Mac
  (8 GB, 16 GB, 24 GB, 32 GB, 48 GB+). Opt-in quality models re-index that disk.
────────────────────────────────────────────────────────────────────────────"#,
            chip_label = chip_label,
            model = self.model_identifier,
            arch_label = arch_label,
            memory_label = memory_label,
            tier = self.tier.name(),
            rec_model = self.recommended_model_name(),
            rec_id = self.recommended_model_id(),
            reason = self.recommendation_reason(),
            headroom = self.high_perf_options_reason(),
        )
    }
}

#[cfg(target_os = "macos")]
fn sysctl_u64(name: &str) -> Option<u64> {
    let c_name = std::ffi::CString::new(name).ok()?;
    let mut val: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    let ret = unsafe {
        libc::sysctlbyname(
            c_name.as_ptr(),
            &mut val as *mut _ as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if ret == 0 {
        Some(val)
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn sysctl_u32(name: &str) -> Option<u32> {
    let c_name = std::ffi::CString::new(name).ok()?;
    let mut val: u32 = 0;
    let mut len = std::mem::size_of::<u32>();
    let ret = unsafe {
        libc::sysctlbyname(
            c_name.as_ptr(),
            &mut val as *mut _ as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if ret == 0 {
        Some(val)
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn sysctl_string(name: &str) -> Option<String> {
    let c_name = std::ffi::CString::new(name).ok()?;
    let mut len: libc::size_t = 0;
    let ret = unsafe {
        libc::sysctlbyname(
            c_name.as_ptr(),
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if ret != 0 || len == 0 {
        return None;
    }
    let mut buf = vec![0u8; len];
    let ret = unsafe {
        libc::sysctlbyname(
            c_name.as_ptr(),
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if ret == 0 {
        while buf.last() == Some(&0) {
            buf.pop();
        }
        String::from_utf8(buf).ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_tier() {
        assert_eq!(HardwareProfile::classify_tier(8.0), HardwareTier::Base16Gb);
        assert_eq!(HardwareProfile::classify_tier(16.0), HardwareTier::Base16Gb);
        assert_eq!(HardwareProfile::classify_tier(18.0), HardwareTier::Base16Gb);
        assert_eq!(HardwareProfile::classify_tier(24.0), HardwareTier::Performance24To32Gb);
        assert_eq!(HardwareProfile::classify_tier(32.0), HardwareTier::Performance24To32Gb);
        assert_eq!(HardwareProfile::classify_tier(36.0), HardwareTier::Performance24To32Gb);
        assert_eq!(HardwareProfile::classify_tier(48.0), HardwareTier::Workstation48GbPlus);
        assert_eq!(HardwareProfile::classify_tier(64.0), HardwareTier::Workstation48GbPlus);
        assert_eq!(HardwareProfile::classify_tier(128.0), HardwareTier::Workstation48GbPlus);
    }

    #[test]
    fn test_detect_hardware() {
        let profile = HardwareProfile::detect();
        assert!(profile.total_memory_bytes > 0);
        assert!(profile.total_memory_gb > 0.0);
        assert!(!profile.cpu_brand.is_empty());
        assert!(!profile.model_identifier.is_empty());
        assert!(profile.cpu_cores > 0);

        let report = profile.cli_report();
        assert!(report.contains("Mac Hardware & Memory Profile"));
        assert!(report.contains("Intelligent Model Recommendation"));
        assert!(report.contains(profile.recommended_model_name()));

        let catalog = profile.model_catalog();
        assert_eq!(catalog.len(), 4);
        assert_eq!(catalog[0].id, crate::embedder::MODEL_ID);
        assert_eq!(catalog[0].status, ModelStatus::ActiveDefault);
    }
}
