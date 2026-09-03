# apps/landing-helioslite — HeliosLite landing page

This is the doc-landing + dev/internal surface for HeliosLite.

## Subpath map

| Path | Renders                | Backing repo + branch       |
|------|------------------------|-----------------------------|
| `/`        | Landing — `HeliosLite`, links to `/docs`, `/docs/quickstart`, GitHub | `heliosLite-src`, branch `main` |
| `/docs`        | Doc index                       | `heliosLite-src/docs`, branch `main` |
| `/docs/quickstart` | `docs/QUICKSTART.md`        | same |
| `/docs/updates` | `docs/UPDATE-STRATEGY.md`     | same |
| `/dev`        | Internal dashboard landing (Grafana OTel, QA, dashboards)         | `ops/dashboards/` (private)         |
| `/api/health` | Probe endpoint                  | forward to `https://helioslite.pheno.studio/api/health` |

The path `/dev/*` is **not** served from Vercel — it is gated through
the local Caddy envelope's `phenomonitor` credentials per
`ops/caddy/Caddyfile.helioslite`.

## Deploy

Hosted at:

- Public docs: `https://helioslite.phenotype.space` (Vercel/GitHub Pages target)
- Dev/internal: `https://helioslite.pheno.studio` (Caddy — WSL Hyper-V surface)

Config: `vercel.json` (Vercel), `ops/caddy/Caddyfile.helioslite` (Caddy).
