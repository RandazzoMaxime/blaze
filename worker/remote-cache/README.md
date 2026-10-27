# blaze-gateway

The Cloudflare Worker in front of the `blaze-cache` R2 bucket ([docs/remote-cache.md](../../docs/remote-cache.md),
"The gateway"). v0 serves today's remote layout per tenant: [API-v0.md](API-v0.md). The blob
store beside it (`/has`, `/get`, `/blob`, `/put`, slice 3's server side): [API.md](API.md).

It is TypeScript, not the workers-rs crate the design names: its tests run in workerd on real R2
and KV bindings (`@cloudflare/vitest-plugin`), and a Rust part would need rustc to build its
wasm, since blaze cannot build `wasm32` yet (AGENTS.md, blaze-builds-blaze).

- Test: `npm ci && npm test` (Node 22; in an ix VM, `nix shell nixpkgs#nodejs_22`). `npm run check` typechecks.
- Deploy: `npx wrangler deploy` here. Live at `https://blaze-gateway.indexable.workers.dev`,
  a tenant's base at `/t/<tenant>/v3`.
- Check a deployment: `python3 smoke.py <gateway URL> <private tenant> <write header file> <read header file>`.
- Measure: `bench/remote-cache/gateway_v0.py` (results in `gateway-v0.tsv`) and
  `bench/remote-cache/cas_v1.py`, which also checks the blob store's threat model live (`cas-v1.tsv`).
- D1 `blaze-gateway` holds the blob store's tables: `npx wrangler d1 migrations apply blaze-gateway --remote`
  after adding one under `migrations/`. The Worker secret `GATEWAY_SECRET_1` is the HKDF input of
  every private tenant's key; it has a copy in `ix secret` (`blaze_gateway_secret_1`), and losing
  every copy makes private blobs unfindable.

Tenants and tokens are KV entries in `blaze-gateway-tokens` (shapes in API-v0.md). A token is
stored only as its SHA-256; mint one with `openssl rand -hex 32` piped straight to where it is
kept, and put `token:<sha256 hex>` with `wrangler kv key put --remote`. Revoke by deleting the
entry (about 60 s to reach every location).
