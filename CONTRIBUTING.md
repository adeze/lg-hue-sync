# Contributing

External contributions use pull requests against `main`. Maintainer work remains direct-to-`main`.

Before submitting:

```bash
make release-check
cargo fmt --all -- --check
cargo test
cargo clippy --bin lg-hue-sync -- -D warnings
perl -0777 -ne 'print $1 if /<script>(.*)<\/script>/s' src/web/ui.html | node --check -
```

Run `make build` when changing runtime code, dependencies, cross-build configuration, or packaging.
Never include TV or Bridge addresses, MAC addresses, credentials, tokens, certificate pins, or a
populated `config.json`. Hardware behavior requires focused automated coverage; a maintainer performs
the final rooted-TV and physical-light validation.

Describe AI assistance in the pull request when used. Contributors remain responsible for reviewing
and testing submitted code.
