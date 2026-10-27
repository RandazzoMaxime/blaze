# blaze-gateway

The Cloudflare Worker in front of the `blaze-cache` R2 bucket ([docs/remote-cache.md](../../docs/remote-cache.md),
"The gateway"). v0 serves today's remote layout per tenant: [API-v0.md](API-v0.md).

It is TypeScript, not the workers-rs crate the design names: its tests run in workerd on real R2
and KV bindings (`@cloudflare/vitest-plugin`), and a Rust part would need rustc to build its
wasm, since blaze cannot build `wasm32` yet (AGENTS.md, blaze-builds-blaze).

- Test: `npm ci && npm test` (Node 22; in an ix VM, `nix shell nixpkgs#nodejs_22`). `npm run check` typechecks.
- Deploy: `npx wrangler deploy` here. Live at `https://blaze-gateway.indexable.workers.dev`,
  a tenant's base at `/t/<tenant>/v3`.
- Check a deployment: `python3 smoke.py <gateway URL> <private tenant> <write header file> <read header file>`.
- Measure: `bench/remote-cache/gateway_v0.py`, results in `bench/remote-cache/gateway-v0.tsv`.

Tenants and tokens are KV entries in `blaze-gateway-tokens` (shapes in API-v0.md). A token is
stored only as its SHA-256; mint one with `openssl rand -hex 32` piped straight to where it is
kept, and put `token:<sha256 hex>` with `wrangler kv key put --remote`. Revoke by deleting the
entry (about 60 s to reach every location).
