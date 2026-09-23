---
name: lg-hue-sync-operations
description: Build, transfer, verify, roll back, uninstall, or release lg-hue-sync for rooted LG webOS TVs while preserving credentials and separating local validation from live proof.
---

# lg-hue-sync operations

Read `AGENTS.md`, then the relevant guide:

- Build/transfer/rollback/uninstall/diagnostics: `docs/operations.md`
- Version/tag/release: `docs/releasing.md`
- Hue/Nanoleaf mapping and ownership: `docs/architecture.md`

## Invariants

- Deploy only `armv7-unknown-linux-gnueabi`; never transfer a host binary.
- Treat `webosbrew/native-toolchain` as the reference SDK, but use the documented Debian Buster build until the SDK passes the `getauxval`/`ring` gate.
- Prefer installed `ares-rs-*` aliases for launcher packaging/install and ordinary transfer; retain SSH for root service lifecycle.
- Preserve the TV's paired `config.json` during updates.
- Treat credentials, tokens, pins, IPs, and MACs as private.
- Require explicit direction before TV writes, service restarts, Luna changes, autoroot, or light output.
- Verify local/remote SHA-256, service state, and dashboard response after transfer.
- Verify physical output before claiming capture-to-light success.
- Preserve Hue v2 channel IDs and grouped gradient member identity.
- Work directly on `main`; no branch or PR unless explicitly requested.

Use repository scripts and Make targets. If a script would overwrite configuration or broaden device mutation, fix the script instead of issuing an ad-hoc destructive command.
