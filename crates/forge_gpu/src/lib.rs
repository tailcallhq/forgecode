//! Semantic GPU lane selection for agent-side CUDA work.
//!
//! Lane intent is stable across runtimes; map to `CUDA_VISIBLE_DEVICES` via
//! [`GpuLane::cuda_visible_device_index`] for the WSL/nvidia-smi profile, or
//! [`GpuLane::cuda_visible_device_index_for_profile`] when a runtime adapter
//! inverts indices (e.g. Windows llama.cpp on this host).

use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Agent-side GPU role lanes for the Pheno heterogeneous stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuLane {
    /// RTX 3090 Ti — primary manager / long-context / kernel synthesis.
    Primary3090,
    /// GTX 1080 Ti — helper drafts / router / llama.cpp CUDA12.
    Legacy1080,
}

/// Runtime profiles that may remap lane → device index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CudaIndexProfile {
    /// WSL / nvidia-smi / PyTorch order: 3090=1, 1080=0.
    #[default]
    NvidiaSmiWslPytorch,
    /// Windows llama.cpp on the Pheno host (inverted vs WSL).
    WindowsLlamaCpp,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GpuLaneError {
    #[error("unknown gpu lane: {0}")]
    Unknown(String),
}

impl GpuLane {
    pub fn role(self) -> &'static str {
        match self {
            Self::Primary3090 => "primary",
            Self::Legacy1080 => "helper",
        }
    }

    pub fn cuda_visible_device_index(self) -> u32 {
        self.cuda_visible_device_index_for_profile(CudaIndexProfile::default())
    }

    pub fn cuda_visible_device_index_for_profile(self, profile: CudaIndexProfile) -> u32 {
        match (self, profile) {
            (Self::Primary3090, CudaIndexProfile::NvidiaSmiWslPytorch) => 1,
            (Self::Legacy1080, CudaIndexProfile::NvidiaSmiWslPytorch) => 0,
            (Self::Primary3090, CudaIndexProfile::WindowsLlamaCpp) => 0,
            (Self::Legacy1080, CudaIndexProfile::WindowsLlamaCpp) => 1,
        }
    }
}

impl FromStr for GpuLane {
    type Err = GpuLaneError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Primary3090" | "primary3090" | "primary" => Ok(Self::Primary3090),
            "Legacy1080" | "legacy1080" | "helper" => Ok(Self::Legacy1080),
            other => Err(GpuLaneError::Unknown(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsl_profile_matches_pheno_harness_policy() {
        assert_eq!(GpuLane::Primary3090.cuda_visible_device_index(), 1);
        assert_eq!(GpuLane::Legacy1080.cuda_visible_device_index(), 0);
    }

    #[test]
    fn windows_llama_profile_inverts_indices() {
        let profile = CudaIndexProfile::WindowsLlamaCpp;
        assert_eq!(
            GpuLane::Primary3090.cuda_visible_device_index_for_profile(profile),
            0
        );
        assert_eq!(
            GpuLane::Legacy1080.cuda_visible_device_index_for_profile(profile),
            1
        );
    }

    #[test]
    fn parses_registry_lane_names() {
        assert_eq!("Primary3090".parse(), Ok(GpuLane::Primary3090));
        assert_eq!("Legacy1080".parse(), Ok(GpuLane::Legacy1080));
    }
}
