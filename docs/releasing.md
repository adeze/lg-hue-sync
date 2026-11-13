# Release workflow

Use Semantic Versioning and direct-to-`main` development.

1. Patch = compatible fix; minor = compatible feature; major = incompatible configuration, protocol, or lifecycle change.
2. Update `Cargo.toml`, `Cargo.lock`, `webos-app/appinfo.json`, IPK metadata, and `CHANGELOG.md` together.
3. Run `make release-check`, `make check`, and `make build` (native default) from [operations.md](operations.md). Run `make build-docker` for the CI/release fallback. Validate each artifact independently; a tested native artifact does not validate a different Docker/Zig binary.
4. Run `make package-app` with Rust Ares installed and `uv run python scripts/test_package_ipk.py`; inspect filename, archive/control metadata and manifest hash. Also run `uv run python scripts/test_native_toolchain.py`. Keep launcher-only packages/manifests local.
5. Commit and push directly to `main`.
6. After the pushed commit and artifacts are verified:

```bash
git tag -a vX.Y.Z -m "lg-hue-sync X.Y.Z"
git push origin vX.Y.Z
```

Pull requests and tags run the same CI gates as `main`. A matching `vX.Y.Z` tag publishes the
verified ARMv7 daemon and `SHA256SUMS` to a GitHub release after both host and target jobs pass.
The workflow explicitly uses `make build-docker` on Linux and publishes the
matching changelog entry as release notes. It intentionally does not publish the
current launcher-only IPK or its manifest. Verify the remote tag/commit, successful
CI, asset SHA-256 and release notes before claiming publication complete.

Do not call a release device-validated without matching binary digests, service/dashboard health, and a physical capture-to-light check. Homebrew Channel publication additionally needs a complete lifecycle and a release manifest matching `webosbrew/apps-repo`.

HACS validates custom repositories against the first non-prerelease in GitHub's release list. For an integration-only release, set the integration manifest version and use that same version as the tag without a leading `v` (for example, `1.3.0`). Publish a regular GitHub release with `--latest=false` so HACS can install its source while GitHub's Latest badge and link still point to the verified TV binary. Publish the integration release without TV assets. Run the [Home Assistant smoke test](../integrations/home-assistant/smoke_test.py) before publishing.
