# OmniRoute Benchmark Plan

**Created:** 2026-05-05
**Status:** Draft
**Session:** 9d873d05

## Overview

Benchmark plan for comparing OmniRoute implementations: TypeScript vs Rust/Go performance.

## Test Scenarios

### 1. Request Routing Performance
- [ ] Single route resolution (no model selection)
- [ ] Multi-route resolution with fallback
- [ ] Concurrent request handling (100/500/1000 RPS)

### 2. Model Selection Latency
- [ ] Token counting overhead
- [ ] Cost calculation per provider
- [ ] Response time comparison (OpenAI vs Anthropic)

### 3. Provider Fallback Chains
- [ ] Single fallback (1 primary, 1 backup)
- [ ] Multi-fallback (1 primary, 2+ backups)
- [ ] Rate limit handling

### 4. Throughput Benchmarks

| Scenario | TS Target | Rust Target | Go Target |
|----------|-----------|-------------|-----------|
| Route Only | <5ms | <1ms | <2ms |
| With Model Select | <50ms | <10ms | <15ms |
| 100 RPS | <200ms p99 | <50ms p99 | <75ms p99 |
| 500 RPS | <500ms p99 | <100ms p99 | <150ms p99 |

## Test Infrastructure

```
/PhenoLang/omniroute-core/
├── benches/           # Criterion benchmarks
├── benches/suite.rs   # Benchmark suite
└── benches/results/   # Historical results
```

## Execution Commands

```bash
# Run all benchmarks
cd /Users/kooshapari/CodeProjects/Phenotype/repos/PhenoLang/omniroute-core
cargo bench --workspace

# Run specific benchmark
cargo bench routing_single

# Compare with baseline
cargo bench --baseline vs_ts_baseline
```

## Baseline Metrics Location

- TS baseline: `baseline_metrics.json` (from session 48462b3f)
- Results: `benches/results/YYYY-MM-DD/*.json`

## Next Steps

1. Create `benches/` directory structure
2. Add Criterion benchmarks for routing
3. Run initial baseline against TS implementation
4. Document p50/p95/p99 latency targets

## Dependencies

- Rust: `criterion = "0.5"`
- Go: `benchstat` for comparison
- Python: `pytest-benchmark` for TS tests
