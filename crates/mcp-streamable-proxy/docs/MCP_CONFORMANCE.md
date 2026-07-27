# Optional MCP conformance checks

This crate’s default `cargo test` suite does **not** run the official MCP
conformance runner (it needs Node.js and a live HTTP endpoint).

## When to use

After starting a Streamable HTTP proxy that fronts a real or mock MCP backend:

```bash
# Terminal A: run your proxy (example)
# cargo run -p mcp-proxy -- ...

# Terminal B: official conformance (active suite)
npx @modelcontextprotocol/conformance server \
  --url http://127.0.0.1:<proxy-port>/mcp \
  --suite active
```

See <https://github.com/modelcontextprotocol/conformance>.

## What it covers vs unit tests

| Layer | Command | Covers |
|-------|---------|--------|
| Unit | `cargo test -p mcp-streamable-proxy` | Isolation defaults, JSON fixtures, slot APIs |
| Integration | (crate tests / local mock) | Per-session connect counting when available |
| Conformance | `npx @modelcontextprotocol/conformance ...` | Protocol compliance against a live proxy URL |

Do **not** add `loom` for this crate’s async/DashMap paths; use `tokio::test` stress instead.
