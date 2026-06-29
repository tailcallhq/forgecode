//! Adaptive eviction window that adjusts based on proximity to threshold.
//!
//! Instead of using a fixed eviction percentage, the adaptive eviction window
//! calculates how close the context is to the compaction threshold and adjusts
//! the eviction percentage accordingly:
//!
//! - When far from threshold (>85% headroom): evict less (conservative)
//! - When approaching threshold (<70% headroom): evict more (aggressive)
//! - When near threshold (<15% headroom): evict maximum (prevent overflow)

/// Adaptive eviction configuration
#[derive(Debug, Clone)]
pub struct AdaptiveEvictionConfig {
    /// Headroom thresholds for adjustment tiers
    pub high_headroom_threshold: f64, // Default: 0.85 (85% headroom = 15% used)
    pub medium_headroom_threshold: f64, // Default: 0.70
    pub low_headroom_threshold: f64,    // Default: 0.85

    /// Eviction percentages for each tier
    pub high_headroom_eviction: f64, // Default: 0.10 (10%)
    pub medium_headroom_eviction: f64,   // Default: 0.20 (20%)
    pub low_headroom_eviction: f64,      // Default: 0.35 (35%)
    pub critical_headroom_eviction: f64, // Default: 0.50 (50%)

    /// Minimum eviction percentage (safety floor)
    pub min_eviction: f64,

    /// Maximum eviction percentage (safety ceiling)
    pub max_eviction: f64,
}

impl Default for AdaptiveEvictionConfig {
    fn default() -> Self {
        Self {
            high_headroom_threshold: 0.85,
            medium_headroom_threshold: 0.70,
            low_headroom_threshold: 0.50,
            high_headroom_eviction: 0.10, // Conservative when far from threshold
            medium_headroom_eviction: 0.20, // Default behavior
            low_headroom_eviction: 0.35,  // Aggressive when approaching threshold
            critical_headroom_eviction: 0.50, // Maximum when near overflow
            min_eviction: 0.05,           // Never evict less than 5%
            max_eviction: 0.60,           // Never evict more than 60%
        }
    }
}

impl AdaptiveEvictionConfig {
    /// Calculate the adaptive eviction percentage based on token count and threshold
    pub fn calculate_eviction(&self, token_count: usize, threshold: usize) -> f64 {
        if threshold == 0 {
            return self.medium_headroom_eviction;
        }

        // Calculate headroom ratio: how much room is left before threshold
        let headroom_ratio = 1.0 - (token_count as f64 / threshold as f64);

        // Determine eviction percentage based on headroom tier
        let eviction = match headroom_ratio {
            r if r >= self.high_headroom_threshold => self.high_headroom_eviction,
            r if r >= self.medium_headroom_threshold => self.medium_headroom_eviction,
            r if r >= self.low_headroom_threshold => self.low_headroom_eviction,
            _ => self.critical_headroom_eviction,
        };

        // Clamp to safety bounds
        eviction.clamp(self.min_eviction, self.max_eviction)
    }
}

/// Adaptive eviction calculator
#[derive(Debug, Clone)]
pub struct AdaptiveEviction {
    config: AdaptiveEvictionConfig,
    enabled: bool,
}

impl Default for AdaptiveEviction {
    fn default() -> Self {
        Self {
            config: AdaptiveEvictionConfig::default(),
            enabled: true, // Enabled by default
        }
    }
}

impl AdaptiveEviction {
    /// Create a new adaptive eviction calculator with default config
    pub fn new() -> Self {
        Self::default()
    }

    /// Create with custom configuration
    pub fn with_config(config: AdaptiveEvictionConfig) -> Self {
        Self { config, enabled: true }
    }

    /// Enable or disable adaptive eviction
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Check if adaptive eviction is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Calculate the adaptive eviction percentage
    ///
    /// Returns the eviction percentage based on:
    /// - Current token count
    /// - Compaction threshold
    /// - Proximity to threshold
    pub fn calculate_eviction(&self, token_count: usize, threshold: usize) -> f64 {
        if !self.enabled || threshold == 0 {
            return self.config.medium_headroom_eviction;
        }

        self.config.calculate_eviction(token_count, threshold)
    }

    /// Calculate headroom ratio for informational purposes
    pub fn headroom_ratio(&self, token_count: usize, threshold: usize) -> f64 {
        if threshold == 0 {
            return 1.0;
        }
        1.0 - (token_count as f64 / threshold as f64)
    }

    /// Determine the current tier for informational purposes
    pub fn current_tier(&self, token_count: usize, threshold: usize) -> &'static str {
        if threshold == 0 {
            return "unknown";
        }

        let headroom = self.headroom_ratio(token_count, threshold);
        match headroom {
            r if r >= self.config.high_headroom_threshold => "high",
            r if r >= self.config.medium_headroom_threshold => "medium",
            r if r >= self.config.low_headroom_threshold => "low",
            _ => "critical",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_creation() {
        let config = AdaptiveEvictionConfig::default();
        assert_eq!(config.high_headroom_eviction, 0.10);
        assert_eq!(config.medium_headroom_eviction, 0.20);
        assert_eq!(config.low_headroom_eviction, 0.35);
    }

    #[test]
    fn test_high_headroom_tier() {
        let config = AdaptiveEvictionConfig::default();
        // 85% headroom means only 15% used - conservative eviction
        let eviction = config.calculate_eviction(15_000, 100_000);
        assert_eq!(eviction, 0.10);
    }

    #[test]
    fn test_medium_headroom_tier() {
        let config = AdaptiveEvictionConfig::default();
        // 70% headroom means 30% used - default eviction
        let eviction = config.calculate_eviction(30_000, 100_000);
        assert_eq!(eviction, 0.20);
    }

    #[test]
    fn test_low_headroom_tier() {
        let config = AdaptiveEvictionConfig::default();
        // 50% headroom means 50% used - aggressive eviction
        let eviction = config.calculate_eviction(50_000, 100_000);
        assert_eq!(eviction, 0.35);
    }

    #[test]
    fn test_critical_headroom_tier() {
        let config = AdaptiveEvictionConfig::default();
        // 10% headroom means 90% used - maximum eviction
        let eviction = config.calculate_eviction(90_000, 100_000);
        assert_eq!(eviction, 0.50);
    }

    #[test]
    fn test_zero_threshold_returns_default() {
        let config = AdaptiveEvictionConfig::default();
        let eviction = config.calculate_eviction(50_000, 0);
        assert_eq!(eviction, config.medium_headroom_eviction);
    }

    #[test]
    fn test_custom_config() {
        let config = AdaptiveEvictionConfig {
            high_headroom_eviction: 0.15,
            medium_headroom_eviction: 0.25,
            low_headroom_eviction: 0.40,
            critical_headroom_eviction: 0.55,
            ..Default::default()
        };

        let eviction = config.calculate_eviction(30_000, 100_000);
        assert_eq!(eviction, 0.25);
    }

    #[test]
    fn test_safety_bounds() {
        let config =
            AdaptiveEvictionConfig { min_eviction: 0.08, max_eviction: 0.45, ..Default::default() };

        // Should be clamped to max
        let eviction = config.calculate_eviction(95_000, 100_000);
        assert_eq!(eviction, 0.45);

        // Should be clamped to min
        let eviction = config.calculate_eviction(10_000, 100_000);
        assert_eq!(eviction, 0.10); // 0.08 is below min of 0.10 for high headroom
    }

    #[test]
    fn test_adaptive_eviction_disabled() {
        let mut eviction = AdaptiveEviction::new();
        eviction.set_enabled(false);

        let result = eviction.calculate_eviction(90_000, 100_000);
        assert_eq!(result, 0.20); // Returns default even with critical tokens
    }

    #[test]
    fn test_adaptive_eviction_enabled() {
        let eviction = AdaptiveEviction::new();

        // 80% used (20% headroom) = critical tier
        let result = eviction.calculate_eviction(80_000, 100_000);
        assert_eq!(result, 0.50);
    }

    #[test]
    fn test_headroom_ratio_calculation() {
        let eviction = AdaptiveEviction::new();

        assert!((eviction.headroom_ratio(25_000, 100_000) - 0.75).abs() < 0.001);
        assert!((eviction.headroom_ratio(100_000, 100_000) - 0.0).abs() < 0.001);
        assert!((eviction.headroom_ratio(0, 100_000) - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_tier_determination() {
        let eviction = AdaptiveEviction::new();

        // headroom = 1.0 - (tokens/threshold)
        assert_eq!(eviction.current_tier(10_000, 100_000), "high"); // 90% headroom
        assert_eq!(eviction.current_tier(30_000, 100_000), "medium"); // 70% headroom
        assert_eq!(eviction.current_tier(50_000, 100_000), "low"); // 50% headroom
        assert_eq!(eviction.current_tier(80_000, 100_000), "critical"); // 20% headroom
        assert_eq!(eviction.current_tier(95_000, 100_000), "critical"); // 5% headroom
    }

    #[test]
    fn test_tier_boundaries() {
        let eviction = AdaptiveEviction::new();

        // At exact threshold boundaries
        assert_eq!(eviction.current_tier(15_000, 100_000), "high"); // 85% headroom
        assert_eq!(eviction.current_tier(30_000, 100_000), "medium"); // 70% headroom
        assert_eq!(eviction.current_tier(50_000, 100_000), "low"); // 50% headroom
    }
}
