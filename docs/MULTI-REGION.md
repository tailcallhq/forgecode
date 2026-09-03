# Multi-Region Deployment

This document covers the architecture, strategy, and operational procedures for ForgeCode's multi-region deployment.

## Architecture Overview

ForgeCode deploys across three primary regions with active-active routing:

```
                        ┌──────────────┐
                        │   DNS/GLB    │
                        │  (Route 53)  │
                        └──────┬───────┘
                 ┌─────────────┼─────────────┐
                 ▼             ▼             ▼
          ┌────────────┐ ┌────────────┐ ┌────────────┐
          │ US East-1  │ │ EU West-1  │ │AP SE-1     │
          │ (Virginia) │ │ (Ireland)  │ │(Singapore) │
          └─────┬──────┘ └─────┬──────┘ └─────┬──────┘
                │              │              │
         ┌──────┴──────┐ ┌────┴────┐ ┌───────┴──────┐
         │  App Nodes  │ │App Nodes│ │  App Nodes   │
         │  OTel Col.  │ │OTel Col.│ │  OTel Col.   │
         │  Prometheus │ │Prometh. │ │  Prometheus  │
         └──────┬──────┘ └────┬────┘ └───────┬──────┘
                │              │              │
         ┌──────▼──────────────▼──────────────▼──────┐
         │          Cross-Region Replication          │
         │    (Datastore + OTel Collector Federation) │
         └───────────────────────────────────────────┘
```

Each region runs a complete service stack: application nodes, an OpenTelemetry Collector for telemetry pipeline, Prometheus for metrics storage, and a regional trace/log backend. A global overlay in Grafana merges regional data for unified dashboards.

## Region Selection Strategy

### Primary Routing (DNS-based)

Requests are routed to the nearest region using latency-based DNS resolution:

| Region | Code | Primary Audience | Data Residency |
|--------|------|-----------------|----------------|
| US East (Virginia) | `us-east-1` | Americas | United States |
| EU West (Ireland) | `eu-west-1` | Europe, Middle East, Africa | EU (GDPR) |
| Asia Pacific (Singapore) | `ap-southeast-1` | Asia Pacific | APAC |

### Selection Algorithm

1. **Geo-proximity**: DNS returns the closest healthy region based on the client's resolver location.
2. **Health weighting**: Regions with degraded health receive reduced DNS weight.
3. **Capacity balancing**: When regions approach saturation (configurable threshold, default 80% CPU), traffic shifts proportionally to healthy neighbors.
4. **Sticky sessions**: Stateful requests use a consistent-hash cookie to pin to a single region for the session duration.

### Priority Hierarchy

Regions have assigned priorities for deterministic failover:

- `us-east-1`: Priority 1 (primary)
- `eu-west-1`: Priority 2 (secondary)
- `ap-southeast-1`: Priority 3 (tertiary)

## Data Replication

### Application Data

| Data Type | Replication Strategy | RPO | RTO |
|-----------|---------------------|-----|-----|
| User accounts | Active-active (CRDTs) | 0 (conflict-free) | < 1s |
| Session state | Regional with cross-region sync | < 5s | < 30s |
| File uploads | Regional primary, async replication | < 60s | < 5 min |
| Audit logs | Regional write, eventual consistency | < 30s | < 2 min |
| Configuration | Global leader (us-east-1), regional read cache | < 10s | < 1 min |

### Replication Topology

```
us-east-1 ◄──── bi-directional ────► eu-west-1
    │                                      │
    └──── bi-directional ────► ap-southeast-1
```

All inter-region replication uses TLS-encrypted channels. Consistency model is **eventual** for non-critical data and **strong** for user-facing writes (via synchronous cross-region commit for writes that affect SLA).

### Conflict Resolution

- **Last-writer-wins (LWW)**: Audit logs, non-critical metadata.
- **CRDTs**: User preferences, counters, collaborative state.
- **Application-level merge**: Business-critical entities use domain-specific merge functions defined in `crates/*/src/merge.rs`.

## Failover Procedures

### Automatic Failover

The system performs automatic failover when a region becomes unhealthy:

1. **Detection**: Each region's health is checked every 10 seconds via OTel Collector health probes.
2. **Threshold**: A region is marked unhealthy after 3 consecutive failures (30 seconds).
3. **DNS update**: Route 53 health checks trigger automatic DNS weight adjustment, steering traffic away from the unhealthy region.
4. **Connection draining**: Existing connections are allowed to complete (configurable drain period, default 60 seconds).
5. **Capacity ramp-up**: Healthy regions scale up autoscaling groups to absorb redistributed traffic.

### Manual Failover

For planned maintenance or controlled failover:

```bash
# 1. Reduce DNS weight for target region to 0
aws route53 change-resource-record-sets \
  --hosted-zone-id Z1234 \
  --change-batch '{"Changes":[{"Action":"UPSERT","ResourceRecordSet":...}]}'

# 2. Verify traffic shift via dashboard
# Check forgecode.dashboard -> Throughput panel filtered by region

# 3. Confirm no errors in receiving regions
# Check forgecode.dashboard -> Errors panel
```

### Post-Failover Validation

After any failover event, verify:

- [ ] Error rate remains below threshold (< 5%)
- [ ] P99 latency is within SLO (< 1000ms)
- [ ] All health checks pass across remaining regions
- [ ] Cross-region replication resumes and catch-up completes
- [ ] Incident ticket is created with timeline and impact assessment

## Latency Considerations

### Inter-Region Latency Baseline

| Path | RTT (typical) | RTT (p99) |
|------|--------------|-----------|
| US East <-> EU West | 75ms | 120ms |
| US East <-> AP SE-1 | 180ms | 260ms |
| EU West <-> AP SE-1 | 160ms | 230ms |

### Optimization Strategies

1. **Regional data locality**: User data is served from the nearest region. Cross-region calls are avoided for hot paths.
2. **Read replicas**: Read-heavy workloads use regional read replicas. Writes propagate asynchronously.
3. **Edge caching**: Static assets and API responses with `Cache-Control` headers are served from Cloudflare edge locations, reducing origin latency.
4. **Connection pooling**: Per-region connection pools to databases and downstream services minimize connection establishment overhead.
5. **Preflight health checks**: Before routing to a region, DNS preflight checks ensure the target is responsive.

### Latency Budget

The total request budget is allocated as follows (p99 target: 1000ms):

| Component | Budget |
|-----------|--------|
| DNS resolution | 20ms |
| TLS handshake (warm) | 10ms |
| Application processing | 300ms |
| Database query (regional) | 100ms |
| External service calls | 200ms |
| Response serialization | 50ms |
| Network jitter buffer | 120ms |
| **Total** | **800ms** (200ms headroom) |

## Compliance Requirements

### Data Residency

| Regulation | Requirement | Implementation |
|-----------|-------------|----------------|
| GDPR (EU) | EU user data must stay in EU | `eu-west-1` handles all EU traffic; data locality enforced at application layer |
| CCPA (California) | California user data protections | US region handles CA traffic; deletion requests propagate globally |
| SOC 2 | Audit trail retention | Logs retained 90 days minimum across all regions; cross-region audit sync |
| ISO 27001 | Information security management | Encryption at rest and in transit; regional key management |

### Compliance Controls

1. **Regional data tagging**: All data records include a `data_residency_region` tag. The application rejects writes that would store EU data outside `eu-west-1`.
2. **Encryption**: AES-256 at rest; TLS 1.3 in transit. Regional KMS keys per deployment.
3. **Access logs**: All cross-region data transfers are logged and auditable.
4. **Data deletion**: Global deletion requests propagate within 24 hours across all regions (SOX/CCPA).
5. **Audit trail**: Immutable audit logs are written to all three regions simultaneously for tamper-evidence.

### Certification Maintenance

- **SOC 2 Type II**: Annual audit; evidence collected from regional deployments.
- **ISO 27001**: Surveillance audit annually; full recertification every 3 years.
- **GDPR**: DPO reviews cross-region data flows quarterly.
