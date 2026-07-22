# nuwax-deploy-installer

Unified deployment CLI for nuwax services. **Binaries are bundled inside this npm package** (no GitHub Release download).

## Quick start (Mac Apple Silicon)

```bash
npm install -g nuwax-deploy-installer@beta
deploy-installer document-parser install --install-dir ~/document-parser
deploy-installer voice-cli install --install-dir ~/voice-cli
```

See [mac-mini-quickstart.md](../../crates/deploy-installer/doc/mac-mini-quickstart.md).

## Commands

```bash
deploy-installer doctor
deploy-installer document-parser setup --install-dir ~/document-parser
deploy-installer document-parser install --install-dir ~/document-parser
deploy-installer document-parser service status --install-dir ~/document-parser
deploy-installer voice-cli setup --install-dir ~/voice-cli
deploy-installer voice-cli service install --install-dir ~/voice-cli
```

## Supported platforms (phase 1)

- macOS Apple Silicon (`darwin-arm64`)
