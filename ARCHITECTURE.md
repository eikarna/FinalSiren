# Architecture & Optimization Notes

## Free Tier Constraints (Cloudflare Workers)
- CPU time per request: ~10 ms (Free Tier)
- No Durable Objects (persistent connections require upgrade)
- Smart placement (`[placement] mode = "smart"`) minimizes latency by routing to nearest data center.

## Upgrade Path for Very Low Ping
- **Durable Objects**: Enables persistent WebSocket/state connections, reducing reconnect overhead significantly.
- **Pro / Enterprise tier**: Higher CPU time limits and Durable Objects support.
- **Cache optimization**: Increase `cache_ttl` for static assets (images already set to 86400).

## Protocol Performance
- All protocols (Vless, VMess, Trojan, ShadowSocks) use TCP/UDP split detection.
- Protocol detection relies on initial 62-byte peek; faster detection = lower ping.
- AEAD operations (VMess) are CPU-intensive; reducing allocations helps within Free Tier limits.

## Health / Connection Tuning
- Default timeout reduced from 3000ms to 2000ms for faster sweep response.
- Bounded concurrency (`buffer_unordered`) prevents exceeding Workers socket limits.
