# blaze-gateway API: the blob store (slice 3 of docs/remote-cache.md)

The content-addressed store beside v0's per-tenant layout (API-v0.md): `/has`, `/get`,
`/blob/<sha256>` and `/put` on the gateway's root. Heads, segments and trust lists
(`/t/<tenant>/heads`, `seg`, `trust`) are slice 4 and not here yet.

A blob is named by the lowercase hex SHA-256 of its **uncompressed** bytes, at most 4 MiB of them.

## Callers and namespaces

Auth is v0's: `Authorization: Bearer <token>`, the KV records of API-v0.md, 401 for an unknown or
expired token, never a fall back to anonymous. The namespaces a request may read are fixed from
the token before any lookup:

| caller | reads | `/put` writes into |
|---|---|---|
| anonymous | public | 401 |
| a public tenant's token | public | public (write token), 403 (read token) |
| a private tenant's token | its own, then public | its own (write token), 403 (read token) |

The public namespace's keys are blob hashes. A private tenant's keys are `HMAC-SHA256(k_t, hash)`,
`k_t = HKDF-SHA256(ikm: GATEWAY_SECRET_<v>, salt: the tenant's 32-byte salt, info: "blaze tenant
key" 0x00 <tenant id> 0x00 <v>)`, with the salt and `v` in D1's `tenants` row (made at the
tenant's first use, salt from SQLite's `randomblob`). A hash another tenant stored privately is
absent to everyone else by the same code path an unknown hash takes.

## Bodies

Requests and answers other than pack streams and blob bytes are deterministic CBOR (RFC 8949
section 4.2.1: shortest integer forms, definite lengths, map keys in bytewise order of their
encodings), `content-type: application/cbor`. A hash in CBOR is a 32-byte byte string.

## Pack stream

`/put`'s body and `/get`'s answer, `content-type: application/vnd.blaze.pack-stream`:

    magic    8 bytes  "BLZSTRM1"
    record*  one of:
      0x01 blob     hash[32] flags[1] (dict[32] when flags bit 2) ulen[u32 LE] clen[u32 LE] data[clen]
      0x02 missing  hash[32]                                    (/get only)
    0x00 end                                                    (a stream without it was cut short)

`flags` bits 0-1 are the codec: 0 is one zstd frame, 1 is raw bytes (`clen == ulen`). Bit 2 says a
dictionary hash follows; the gateway refuses dictionaries until slice 6 (422). Other bits are 0.
`ulen` is at most 4 MiB.

## Requests

| request | answer |
|---|---|
| `POST /has` `{"hashes": [h, ...]}` (at most 20,000) | 200 `{"state": [s, ...]}`, one per hash in order: 0 absent, 1 public, 2 in the caller's own namespace (also when public) |
| `POST /get` `{"hashes": [h, ...]}` (at most 20,000) | 200, a pack stream: a `blob` record for each hash found, in the order the gateway reads them (own namespace first), then a `missing` record for each other hash, then `end`. The records are the stored frames, as `/put` received them |
| `GET` / `HEAD /blob/<h>` (optional `Range: bytes=a-` or `bytes=a-b`) | 200 / 206: the stored frame (zstd or raw), `x-blaze-codec: zstd\|raw`, `x-blaze-ulength`; the one 404 when absent from every namespace the caller may read; 416 |
| `POST /put` a pack stream (at most 32 MiB, `Content-Length` required) | 200 `{"pack": "<hex>", "blobs": n}` after the pack is stored and every location recorded |

`/put` decompresses every zstd frame in the Worker (workerd's `node:zlib` zstd, the C library in
the runtime) and refuses the whole request, storing nothing, when a frame does not hash to its
name, its length is not its `ulen`, a `ulen` is over 4 MiB, or a frame carries a dictionary: 422
with the first offending hash. A malformed or truncated stream is 400. Every accepted `/put` is
stored in full as one new pack, even when every blob in it is already present, so neither its
answer nor its timing depends on what is stored. Each `(namespace, key)` has one location, the
newest copy.

## Storage

A `/put` becomes one object, `cas/pub/<sha256 of the pack>.bpk` or, for a private tenant,
`cas/t/<tenant id>/<HMAC(k_t, sha256 of the pack)>.bpk`:

    header  "BLZPACK1"
    frames  each blob's data, back to back
    index   CBOR array of [key, offset, clen, ulen, codec], key = the namespace's key for the blob
    footer  index offset [u64 LE], index length [u64 LE], "BLZPACK1"

D1 (`migrations/`): `tenants(id, salt, key_version)`, `objects(object, uploaded)` and
`blobs(ns, key, object, offset, length, ulength, codec, dict)` keyed by `(ns, key)`, `ns` being
`pub` or the tenant id, `key` the hex key. A private pack's object name, its index and its D1
rows hold HMACs only, never a blob's hash.
