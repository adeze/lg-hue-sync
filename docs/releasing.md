# Release workflow

Use Semantic Versioning and direct-to-`main` development.

1. Patch = compatible fix; minor = compatible feature; major = incompatible configuration, protocol, or lifecycle change.
2. Update `Cargo.toml`, `Cargo.lock`, `webos-app/appinfo.json`, IPK metadata, and `CHANGELOG.md` together.
3. Run `make release-check`, then the local gate and target build from [operations.md](operations.md).
4. Build the IPK with `uv run scripts/package_ipk.py`; inspect its filename and control metadata.
5. Commit and push directly to `main`.
6. After the pushed commit and artifacts are verified:

```bash
git tag -a vX.Y.Z -m "lg-hue-sync X.Y.Z"
git push origin vX.Y.Z
```

Pull requests and tags run the same CI gates as `main`. A matching `vX.Y.Z` tag publishes the
verified ARMv7 daemon and `SHA256SUMS` to a GitHub release after both host and target jobs pass.
The workflow intentionally does not publish the current launcher-only IPK or its manifest.

Do not call a release device-validated without matching binary digests, service/dashboard health, and a physical capture-to-light check. Homebrew Channel publication additionally needs a complete lifecycle and a release manifest matching `webosbrew/apps-repo`.
